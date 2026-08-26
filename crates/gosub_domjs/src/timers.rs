//! A virtual-time timer queue.
//!
//! There is no real clock and no event loop: the driver pumps the queue after the page's
//! scripts have run, and each callback advances a virtual "now" to its own due time. That is
//! enough for testharness's `step_timeout` and for `requestAnimationFrame`, and it means a
//! test never waits on wall-clock time.

use std::cell::RefCell;
use std::rc::Rc;

use rquickjs::prelude::{Func, Opt, Rest};
use rquickjs::{Array, Ctx, Function, Object, Result, Value};

/// Callbacks live in a JS object rather than in Rust so the GC keeps them alive.
const CALLBACKS: &str = "__gosub_timer_callbacks";

/// One frame, for `requestAnimationFrame`. Nothing here paints; it is just the delay that
/// makes rAF resolve in a pumped queue.
const FRAME_MS: f64 = 16.0;

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Timeout,
    Frame,
}

struct Timer {
    id: u32,
    due: f64,
    /// Insertion order, so timers with the same due time fire in the order they were set.
    seq: u64,
    repeat: Option<f64>,
    kind: Kind,
}

#[derive(Default)]
pub struct TimerState {
    now: f64,
    seq: u64,
    next_id: u32,
    timers: Vec<Timer>,
}

impl TimerState {
    fn schedule(&mut self, delay: f64, repeat: Option<f64>, kind: Kind) -> u32 {
        self.next_id += 1;
        self.seq += 1;
        self.timers.push(Timer {
            id: self.next_id,
            due: self.now + delay.max(0.0),
            seq: self.seq,
            repeat,
            kind,
        });
        self.next_id
    }

    fn cancel(&mut self, id: u32) {
        self.timers.retain(|t| t.id != id);
    }

    /// Index of the timer that fires next: earliest due time, then insertion order.
    fn next_due(&self) -> Option<usize> {
        self.timers
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| a.due.total_cmp(&b.due).then(a.seq.cmp(&b.seq)))
            .map(|(index, _)| index)
    }

    pub fn pending(&self) -> usize {
        self.timers.len()
    }
}

pub type Timers = Rc<RefCell<TimerState>>;

fn store_callback<'js>(ctx: &Ctx<'js>, id: u32, callback: Function<'js>, args: Vec<Value<'js>>) -> Result<()> {
    let callbacks: Object<'js> = ctx.globals().get(CALLBACKS)?;
    let entry = Object::new(ctx.clone())?;
    entry.set("callback", callback)?;
    let array = Array::new(ctx.clone())?;
    for (index, arg) in args.into_iter().enumerate() {
        array.set(index, arg)?;
    }
    entry.set("args", array)?;
    callbacks.set(id, entry)
}

fn take_callback<'js>(ctx: &Ctx<'js>, id: u32, keep: bool) -> Result<Option<(Function<'js>, Array<'js>)>> {
    let callbacks: Object<'js> = ctx.globals().get(CALLBACKS)?;
    let Some(entry) = callbacks.get::<_, Option<Object<'js>>>(id)? else {
        return Ok(None);
    };
    let callback: Function<'js> = entry.get("callback")?;
    let args: Array<'js> = entry.get("args")?;
    if !keep {
        callbacks.remove(id)?;
    }
    Ok(Some((callback, args)))
}

/// Pin every argument of a closure to the same `'js` lifetime. Without this the compiler
/// gives each parameter its own, and `Ctx`/`Function` are invariant, so nothing type-checks.
fn schedule_fn<F>(f: F) -> F
where
    F: for<'js> Fn(Ctx<'js>, Function<'js>, Opt<Value<'js>>, Rest<Value<'js>>) -> Result<u32>,
{
    f
}

/// testharness passes `null` for both delays and timer ids, which is not the same thing as
/// omitting the argument - take the raw value and coerce it here.
fn as_number(value: Opt<Value<'_>>) -> Option<f64> {
    let value = value.0?;
    value.as_number().or_else(|| value.as_int().map(f64::from))
}

fn clear_fn<F>(f: F) -> F
where
    F: for<'js> Fn(Opt<Value<'js>>),
{
    f
}

fn frame_fn<F>(f: F) -> F
where
    F: for<'js> Fn(Ctx<'js>, Function<'js>) -> Result<u32>,
{
    f
}

/// Install `setTimeout`/`setInterval`/`requestAnimationFrame` and their cancel functions.
pub fn install(ctx: &Ctx<'_>, timers: &Timers) -> Result<()> {
    let globals = ctx.globals();
    globals.set(CALLBACKS, Object::new(ctx.clone())?)?;

    let set_timer = |timers: Timers, repeating: bool| {
        schedule_fn(move |ctx, callback, delay, args: Rest<_>| {
            let delay = as_number(delay).unwrap_or(0.0);
            let repeat = repeating.then_some(delay.max(1.0));
            let id = timers.borrow_mut().schedule(delay, repeat, Kind::Timeout);
            store_callback(&ctx, id, callback, args.0)?;
            Ok(id)
        })
    };

    globals.set("setTimeout", Func::from(set_timer(timers.clone(), false)))?;
    globals.set("setInterval", Func::from(set_timer(timers.clone(), true)))?;

    let clear = |timers: Timers| {
        clear_fn(move |id| {
            if let Some(id) = as_number(id) {
                timers.borrow_mut().cancel(id as u32);
            }
        })
    };
    globals.set("clearTimeout", Func::from(clear(timers.clone())))?;
    globals.set("clearInterval", Func::from(clear(timers.clone())))?;

    let raf_timers = timers.clone();
    globals.set(
        "requestAnimationFrame",
        Func::from(frame_fn(move |ctx, callback| {
            let id = raf_timers.borrow_mut().schedule(FRAME_MS, None, Kind::Frame);
            store_callback(&ctx, id, callback, Vec::new())?;
            Ok(id)
        })),
    )?;
    globals.set("cancelAnimationFrame", Func::from(clear(timers.clone())))?;
    Ok(())
}

/// Fire the timer that is due next, if any, and drain the microtasks it queued.
/// Returns `false` when the queue is empty.
pub fn run_next(ctx: &Ctx<'_>, timers: &Timers) -> Result<bool> {
    let fired = {
        let mut state = timers.borrow_mut();
        let Some(index) = state.next_due() else {
            return Ok(false);
        };
        let timer = state.timers.remove(index);
        state.now = state.now.max(timer.due);

        if let Some(interval) = timer.repeat {
            state.seq += 1;
            let seq = state.seq;
            let due = state.now + interval;
            state.timers.push(Timer { due, seq, ..timer });
        }
        (timer.id, timer.kind, timer.repeat.is_some(), state.now)
    };

    let (id, kind, repeating, now) = fired;
    let Some((callback, args)) = take_callback(ctx, id, repeating)? else {
        return Ok(true);
    };

    let outcome = match kind {
        Kind::Frame => callback.call::<_, Value<'_>>((now,)),
        Kind::Timeout => {
            let mut call_args = rquickjs::function::Args::new(ctx.clone(), args.len());
            call_args.push_args(args.iter::<Value<'_>>().collect::<Result<Vec<_>>>()?)?;
            callback.call_arg::<Value<'_>>(call_args)
        }
    };
    if let Err(e) = outcome {
        // The spec reports the exception and keeps the queue running.
        eprintln!("  [timer] callback threw: {e}");
    }

    while ctx.execute_pending_job() {}
    Ok(true)
}
