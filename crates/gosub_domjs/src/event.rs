//! DOM events: the listener registry and the dispatch algorithm.
//!
//! The propagation path comes from the real document tree, so what a test observes about
//! capture/target/bubble order is the engine's tree, not a JS-side mirror of it.

use gosub_interface::document::Document as _;
use gosub_shared::node::NodeId;
use rquickjs::class::Trace;
use rquickjs::function::This;
use rquickjs::prelude::Opt;
use rquickjs::{Class, Ctx, Function, JsLifetime, Object, Result, Value};

use crate::DocHandle;

/// Listener key for the global object: no node has this id.
pub const WINDOW_KEY: u64 = u64::MAX;

const LISTENERS: &str = "__gosub_listeners";

const PHASE_NONE: u32 = 0;
const PHASE_CAPTURING: u32 = 1;
const PHASE_AT_TARGET: u32 = 2;
const PHASE_BUBBLING: u32 = 3;

#[derive(Trace, JsLifetime)]
struct Listener<'js> {
    #[qjs(skip_trace)]
    key: u64,
    #[qjs(skip_trace)]
    event_type: String,
    #[qjs(skip_trace)]
    capture: bool,
    #[qjs(skip_trace)]
    once: bool,
    /// Removed listeners are tombstoned rather than deleted: dispatch holds indices into
    /// this list and must see removals that happen while it is running.
    #[qjs(skip_trace)]
    removed: bool,
    callback: Function<'js>,
}

#[derive(Trace, JsLifetime)]
#[rquickjs::class(rename = "EventListeners")]
pub struct ListenerStore<'js> {
    entries: Vec<Listener<'js>>,
}

/// How a listener is registered, per the spec's dedup rule: type + callback + capture.
struct Registration<'js> {
    key: u64,
    event_type: String,
    callback: Function<'js>,
    capture: bool,
    once: bool,
}

fn store<'js>(ctx: &Ctx<'js>) -> Result<Class<'js, ListenerStore<'js>>> {
    ctx.globals().get(LISTENERS)
}

/// `options` is either a boolean capture flag or a dictionary.
fn listener_options(options: Opt<Value<'_>>) -> (bool, bool) {
    let Some(value) = options.0 else {
        return (false, false);
    };
    if let Some(capture) = value.as_bool() {
        return (capture, false);
    }
    let Some(object) = value.as_object() else {
        return (false, false);
    };
    let flag = |name: &str| object.get::<_, Option<bool>>(name).ok().flatten().unwrap_or(false);
    (flag("capture"), flag("once"))
}

pub fn add<'js>(
    ctx: &Ctx<'js>,
    key: u64,
    event_type: String,
    callback: Function<'js>,
    options: Opt<Value<'js>>,
) -> Result<()> {
    let (capture, once) = listener_options(options);
    let registration = Registration {
        key,
        event_type,
        callback,
        capture,
        once,
    };

    let store = store(ctx)?;
    let mut store = store.borrow_mut();
    let duplicate = store.entries.iter().any(|e| {
        !e.removed
            && e.key == registration.key
            && e.event_type == registration.event_type
            && e.capture == registration.capture
            && e.callback.as_value() == registration.callback.as_value()
    });
    if duplicate {
        return Ok(());
    }
    store.entries.push(Listener {
        key: registration.key,
        event_type: registration.event_type,
        capture: registration.capture,
        once: registration.once,
        removed: false,
        callback: registration.callback,
    });
    Ok(())
}

pub fn remove<'js>(
    ctx: &Ctx<'js>,
    key: u64,
    event_type: &str,
    callback: &Function<'js>,
    options: Opt<Value<'js>>,
) -> Result<()> {
    let (capture, _) = listener_options(options);
    let store = store(ctx)?;
    let mut store = store.borrow_mut();
    for entry in &mut store.entries {
        if !entry.removed
            && entry.key == key
            && entry.event_type == event_type
            && entry.capture == capture
            && entry.callback.as_value() == callback.as_value()
        {
            entry.removed = true;
            return Ok(());
        }
    }
    Ok(())
}

#[derive(Trace, JsLifetime)]
#[rquickjs::class(rename = "Event")]
pub struct DomEvent<'js> {
    #[qjs(skip_trace)]
    event_type: String,
    #[qjs(skip_trace)]
    bubbles: bool,
    #[qjs(skip_trace)]
    cancelable: bool,
    #[qjs(skip_trace)]
    composed: bool,
    #[qjs(skip_trace)]
    phase: u32,
    #[qjs(skip_trace)]
    stopped: bool,
    #[qjs(skip_trace)]
    stopped_immediately: bool,
    #[qjs(skip_trace)]
    canceled: bool,
    #[qjs(skip_trace)]
    trusted: bool,
    target: Option<Value<'js>>,
    current_target: Option<Value<'js>>,
}

impl<'js> DomEvent<'js> {
    pub fn synthetic(event_type: &str, bubbles: bool, cancelable: bool) -> Self {
        Self {
            event_type: event_type.to_string(),
            bubbles,
            cancelable,
            composed: false,
            phase: PHASE_NONE,
            stopped: false,
            stopped_immediately: false,
            canceled: false,
            trusted: false,
            target: None,
            current_target: None,
        }
    }
}

#[rquickjs::methods(rename_all = "camelCase")]
impl<'js> DomEvent<'js> {
    #[qjs(constructor)]
    pub fn new(event_type: String, init: Opt<Object<'js>>) -> Self {
        let flag = |name: &str| {
            init.0
                .as_ref()
                .and_then(|o| o.get::<_, Option<bool>>(name).ok().flatten())
                .unwrap_or(false)
        };
        let mut event = DomEvent::synthetic(&event_type, flag("bubbles"), flag("cancelable"));
        event.composed = flag("composed");
        event
    }

    #[qjs(get, rename = "type")]
    pub fn event_type(&self) -> String {
        self.event_type.clone()
    }

    #[qjs(get)]
    pub fn bubbles(&self) -> bool {
        self.bubbles
    }

    #[qjs(get)]
    pub fn cancelable(&self) -> bool {
        self.cancelable
    }

    #[qjs(get)]
    pub fn composed(&self) -> bool {
        self.composed
    }

    #[qjs(get)]
    pub fn is_trusted(&self) -> bool {
        self.trusted
    }

    #[qjs(get)]
    pub fn event_phase(&self) -> u32 {
        self.phase
    }

    #[qjs(get)]
    pub fn default_prevented(&self) -> bool {
        self.canceled
    }

    #[qjs(get)]
    pub fn target(&self) -> Option<Value<'js>> {
        self.target.clone()
    }

    #[qjs(get)]
    pub fn current_target(&self) -> Option<Value<'js>> {
        self.current_target.clone()
    }

    #[qjs(get)]
    pub fn time_stamp(&self) -> f64 {
        0.0
    }

    pub fn prevent_default(&mut self) {
        if self.cancelable {
            self.canceled = true;
        }
    }

    pub fn stop_propagation(&mut self) {
        self.stopped = true;
    }

    pub fn stop_immediate_propagation(&mut self) {
        self.stopped = true;
        self.stopped_immediately = true;
    }
}

/// One step of the propagation path: what the listener key is, and what `currentTarget`
/// should be while listeners on it run.
struct PathEntry<'js> {
    key: u64,
    value: Value<'js>,
}

/// Target → ancestors → global object. The document node reports the `document` object
/// rather than a node wrapper, so `event.currentTarget === document` holds.
fn propagation_path<'js>(ctx: &Ctx<'js>, doc: &DocHandle, target: NodeId) -> Result<Vec<PathEntry<'js>>> {
    let mut ids = vec![target];
    let mut current = target;
    while let Some(parent) = doc.borrow().parent(current) {
        ids.push(parent);
        current = parent;
    }

    let root = doc.borrow().root();
    let mut path = Vec::with_capacity(ids.len() + 1);
    for id in ids {
        let value = if id == root {
            ctx.globals().get::<_, Value<'js>>("document")?
        } else {
            crate::wrap(ctx, doc, id)?
        };
        path.push(PathEntry {
            key: u64::from(id),
            value,
        });
    }
    path.push(PathEntry {
        key: WINDOW_KEY,
        value: ctx.globals().into_value(),
    });
    Ok(path)
}

/// Which listeners run at this step: only capture ones, only bubble ones, or both.
#[derive(Clone, Copy, PartialEq)]
enum Which {
    Capture,
    Bubble,
    Both,
}

fn run_listeners<'js>(
    ctx: &Ctx<'js>,
    event: &Class<'js, DomEvent<'js>>,
    entry: &PathEntry<'js>,
    which: Which,
    phase: u32,
) -> Result<()> {
    let event_type = event.borrow().event_type.clone();

    let indices: Vec<usize> = {
        let store = store(ctx)?;
        let store = store.borrow();
        store
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                !e.removed
                    && e.key == entry.key
                    && e.event_type == event_type
                    && match which {
                        Which::Capture => e.capture,
                        Which::Bubble => !e.capture,
                        Which::Both => true,
                    }
            })
            .map(|(index, _)| index)
            .collect()
    };
    if indices.is_empty() {
        return Ok(());
    }

    {
        let mut event_mut = event.borrow_mut();
        event_mut.phase = phase;
        event_mut.current_target = Some(entry.value.clone());
    }

    for index in indices {
        // Re-check: a previous listener may have removed this one.
        let callback = {
            let store = store(ctx)?;
            let mut store = store.borrow_mut();
            let listener = &mut store.entries[index];
            if listener.removed {
                continue;
            }
            if listener.once {
                listener.removed = true;
            }
            listener.callback.clone()
        };

        let event_value = event.clone().into_value();
        if let Err(e) = callback.call::<_, Value<'js>>((This(entry.value.clone()), event_value)) {
            // The spec reports listener exceptions and carries on with the next listener.
            eprintln!("  [event] listener for {event_type:?} threw: {e}");
        }

        if event.borrow().stopped_immediately {
            break;
        }
    }
    Ok(())
}

/// Dispatch `event` at `target`. Returns false when a listener called `preventDefault()`.
pub fn dispatch<'js>(
    ctx: &Ctx<'js>,
    doc: &DocHandle,
    target: NodeId,
    event: Class<'js, DomEvent<'js>>,
) -> Result<bool> {
    let path = propagation_path(ctx, doc, target)?;
    {
        let mut event_mut = event.borrow_mut();
        event_mut.target = Some(path[0].value.clone());
        event_mut.stopped = false;
        event_mut.stopped_immediately = false;
    }

    for entry in path.iter().skip(1).rev() {
        if event.borrow().stopped {
            break;
        }
        run_listeners(ctx, &event, entry, Which::Capture, PHASE_CAPTURING)?;
    }

    if !event.borrow().stopped {
        run_listeners(ctx, &event, &path[0], Which::Both, PHASE_AT_TARGET)?;
    }

    if event.borrow().bubbles {
        for entry in path.iter().skip(1) {
            if event.borrow().stopped {
                break;
            }
            run_listeners(ctx, &event, entry, Which::Bubble, PHASE_BUBBLING)?;
        }
    }

    let mut event_mut = event.borrow_mut();
    event_mut.phase = PHASE_NONE;
    event_mut.current_target = None;
    Ok(!event_mut.canceled)
}

/// Pin every argument of a closure to the same `'js` lifetime - see `timers::schedule_fn`.
fn add_fn<F>(f: F) -> F
where
    F: for<'js> Fn(Ctx<'js>, String, Function<'js>, Opt<Value<'js>>) -> Result<()>,
{
    f
}

/// Install `Event` plus the global object's own `EventTarget` methods.
pub fn install(ctx: &Ctx<'_>) -> Result<()> {
    let globals = ctx.globals();
    globals.set(
        LISTENERS,
        Class::instance(ctx.clone(), ListenerStore { entries: Vec::new() })?,
    )?;
    Class::<DomEvent>::define(&globals)?;

    globals.set(
        "addEventListener",
        rquickjs::prelude::Func::from(add_fn(|ctx, event_type, callback, options| {
            add(&ctx, WINDOW_KEY, event_type, callback, options)
        })),
    )?;
    globals.set(
        "removeEventListener",
        rquickjs::prelude::Func::from(add_fn(|ctx, event_type, callback, options| {
            remove(&ctx, WINDOW_KEY, &event_type, &callback, options)
        })),
    )?;
    Ok(())
}
