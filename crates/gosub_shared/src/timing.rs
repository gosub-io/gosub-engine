use parking_lot::Mutex;
use std::collections::HashMap;
use std::fmt::{Display, Formatter};
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

use lazy_static::lazy_static;
#[cfg(target_arch = "wasm32")]
use web_sys::window;

type TimerId = uuid::Uuid;

fn new_timer_id() -> TimerId {
    uuid::Uuid::new_v4()
}

#[derive(Debug, Clone)]
pub enum Scale {
    MicroSecond,
    MilliSecond,
    Second,
    Auto,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, PartialOrd, Ord)]
#[allow(dead_code)]
struct Duration {
    duration: u64,
    suffix: String,
}

impl Display for Duration {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}{}", self.duration, self.suffix)
    }
}

#[derive(Default, Debug, Clone)]
pub struct TimingTable {
    timers: HashMap<TimerId, Timer>,
    namespaces: HashMap<String, Vec<TimerId>>,
    /// Start of each scope, the epoch its marks are measured from. Held as a running
    /// `Timer` so the platform clock handling stays in one place.
    epochs: HashMap<ScopeId, Timer>,
}

pub struct Stats {
    count: u64,
    total: u64,
    min: u64,
    max: u64,
    avg: u64,
    p50: u64,
    p75: u64,
    p95: u64,
    p99: u64,
}

/// Aggregated timing statistics for a single namespace, suitable for external consumption.
#[derive(Debug, Clone)]
pub struct NamespaceStats {
    pub namespace: String,
    pub count: u64,
    pub total_us: u64,
    pub min_us: u64,
    pub max_us: u64,
    pub avg_us: u64,
    pub p50_us: u64,
    pub p75_us: u64,
    pub p95_us: u64,
    pub p99_us: u64,
}

fn percentage_to_index(count: u64, percentage: f64) -> usize {
    ((count as f64 * percentage) as usize).min(count.saturating_sub(1) as usize)
}

impl TimingTable {
    #[must_use]
    pub fn new() -> TimingTable {
        TimingTable {
            timers: HashMap::new(),
            namespaces: HashMap::new(),
            epochs: HashMap::new(),
        }
    }

    pub fn start_timer(&mut self, namespace: &str, context: Option<String>) -> TimerId {
        let timer = Timer::new(context);
        self.timers.insert(timer.id, timer.clone());
        self.namespaces.entry(namespace.to_string()).or_default().push(timer.id);

        timer.id
    }

    pub fn stop_timer(&mut self, timer_id: TimerId) {
        if let Some(timer) = self.timers.get_mut(&timer_id) {
            timer.end();
        }
    }

    /// Stamp the start of `scope`. Marks recorded against it are measured from here.
    ///
    /// Re-stamping an existing scope resets its epoch, which is what a reload wants.
    pub fn begin_scope(&mut self, scope: ScopeId) {
        self.epochs.insert(scope, Timer::new(None));
    }

    /// Record how far into `scope` we are right now, under `namespace`.
    ///
    /// A mark is not a duration of work - it is a moment in a navigation ("first paint
    /// happened here"). It is still stored as a sample, because the distance from the
    /// navigation's epoch to that moment *is* a duration, so the existing per-namespace
    /// aggregation applies unchanged.
    ///
    /// Does nothing when the scope has no epoch: a mark with no navigation to measure
    /// from is meaningless, and inventing one would read as a real measurement.
    pub fn mark_in(&mut self, scope: ScopeId, namespace: &str, context: Option<String>) {
        let Some(elapsed) = self.epochs.get(&scope).map(Timer::elapsed_us) else {
            return;
        };
        let mut timer = Timer::finished(context, elapsed);
        timer.scope = Some(scope);
        self.timers.insert(timer.id, timer.clone());
        self.namespaces.entry(namespace.to_string()).or_default().push(timer.id);
    }

    /// File an already-finished timer under `namespace`.
    pub fn insert_finished(&mut self, namespace: &str, timer: Timer) {
        self.timers.insert(timer.id, timer.clone());
        self.namespaces.entry(namespace.to_string()).or_default().push(timer.id);
    }

    /// Record a duration that was measured somewhere else.
    ///
    /// Some work is timed by a component we don't drive the clock for - the fetch
    /// stack reports its own elapsed time on `NetEvent::Finished`, for instance. Those
    /// durations still belong in the table, but there is no start/stop pair to hang
    /// them off. This files a already-finished timer directly.
    pub fn record(&mut self, namespace: &str, duration_us: u64, context: Option<String>) {
        let timer = Timer::finished(context, duration_us);
        self.timers.insert(timer.id, timer.clone());
        self.namespaces.entry(namespace.to_string()).or_default().push(timer.id);
    }

    #[must_use]
    pub fn get_stats(&self, timers: &Vec<TimerId>) -> Stats {
        let mut durations: Vec<u64> = Vec::new();

        for timer_id in timers {
            if let Some(timer) = self.timers.get(timer_id) {
                if !timer.has_finished() {
                    continue;
                }
                durations.push(timer.duration_us);
            }
        }

        durations.sort_unstable();
        let count = durations.len() as u64;
        let total: u64 = durations.iter().sum();
        let min = *durations.first().unwrap_or(&0);
        let max = *durations.last().unwrap_or(&0);
        let avg = total.checked_div(count).unwrap_or(0);
        let p50 = durations.get(percentage_to_index(count, 0.50)).copied().unwrap_or(0);
        let p75 = durations.get(percentage_to_index(count, 0.75)).copied().unwrap_or(0);
        let p95 = durations.get(percentage_to_index(count, 0.95)).copied().unwrap_or(0);
        let p99 = durations.get(percentage_to_index(count, 0.99)).copied().unwrap_or(0);

        Stats {
            count,
            total,
            min,
            max,
            avg,
            p50,
            p75,
            p95,
            p99,
        }
    }

    /// Returns aggregated stats for every registered namespace.
    #[must_use]
    pub fn namespace_stats(&self) -> Vec<NamespaceStats> {
        self.namespace_stats_for(None)
    }

    /// Aggregated per-namespace statistics.
    ///
    /// `scope` of `None` aggregates every sample regardless of scope, which is what the
    /// global views (`gosub://stats`, `/metrics`) want. `Some(id)` narrows to one unit of
    /// work. Namespaces with no samples in that scope are omitted rather than reported as
    /// zero, so a per-navigation view only lists what that navigation actually did.
    #[must_use]
    pub fn namespace_stats_for(&self, scope: Option<ScopeId>) -> Vec<NamespaceStats> {
        self.namespaces
            .iter()
            .filter_map(|(ns, timer_ids)| {
                let timer_ids: Vec<TimerId> = match scope {
                    None => timer_ids.clone(),
                    Some(want) => timer_ids
                        .iter()
                        .filter(|id| self.timers.get(id).is_some_and(|t| t.scope == Some(want)))
                        .copied()
                        .collect(),
                };
                if timer_ids.is_empty() {
                    return None;
                }
                let s = self.get_stats(&timer_ids);
                Some(NamespaceStats {
                    namespace: ns.clone(),
                    count: s.count,
                    total_us: s.total,
                    min_us: s.min,
                    max_us: s.max,
                    avg_us: s.avg,
                    p50_us: s.p50,
                    p75_us: s.p75,
                    p95_us: s.p95,
                    p99_us: s.p99,
                })
            })
            .collect()
    }

    /// Clears all recorded timings.
    pub fn clear(&mut self) {
        self.timers.clear();
        self.namespaces.clear();
        self.epochs.clear();
    }

    /// Drop every sample belonging to `scope`, leaving other scopes untouched.
    pub fn clear_scope(&mut self, scope: ScopeId) {
        self.epochs.remove(&scope);
        self.timers.retain(|_, t| t.scope != Some(scope));
        for ids in self.namespaces.values_mut() {
            ids.retain(|id| self.timers.contains_key(id));
        }
        self.namespaces.retain(|_, ids| !ids.is_empty());
    }

    fn scale(&self, value: u64, scale: Scale) -> String {
        match scale {
            Scale::MicroSecond => format!("{value}µs"),
            Scale::MilliSecond => format!("{}ms", value / 1000),
            Scale::Second => format!("{}s", value / (1000 * 1000)),
            Scale::Auto => {
                if value < 1000 {
                    format!("{value}µs")
                } else if value < 1000 * 1000 {
                    format!("{}ms", value / 1000)
                } else {
                    format!("{}s", value / (1000 * 1000))
                }
            }
        }
    }

    pub fn print_timings(&self, show_details: bool, scale: Scale) {
        println!("Namespace            |    Count |      Total |        Min |        Max |        Avg |        50% |        75% |        95% |        99%");
        println!("----------------------------------------------------------------------------------------------------------------------------------------");
        for (namespace, timers) in &self.namespaces {
            let stats = self.get_stats(timers);
            println!(
                "{:20} | {:>8} | {:>10} | {:>10} | {:>10} | {:>10} | {:>10} | {:>10} | {:>10} | {:>10}",
                namespace,
                stats.count,
                self.scale(stats.total, scale.clone()),
                self.scale(stats.min, scale.clone()),
                self.scale(stats.max, scale.clone()),
                self.scale(stats.avg, scale.clone()),
                self.scale(stats.p50, scale.clone()),
                self.scale(stats.p75, scale.clone()),
                self.scale(stats.p95, scale.clone()),
                self.scale(stats.p99, scale.clone()),
            );

            if show_details {
                for timer_id in timers {
                    if let Some(timer) = self.timers.get(timer_id) {
                        if timer.has_finished() {
                            let context = timer.context.clone().unwrap_or_default();
                            if context.is_empty() {
                                continue;
                            }
                            println!(
                                "                     | {:>8} | {:>10} | {}",
                                1,
                                self.scale(timer.duration_us, scale.clone()),
                                context
                            );
                        }
                    }
                }
            }
        }
    }

    #[must_use]
    pub fn duration(&self, timer_id: TimerId) -> u64 {
        if let Some(timer) = self.timers.get(&timer_id) {
            timer.duration()
        } else {
            0
        }
    }
}

lazy_static! {
    pub static ref TIMING_TABLE: Mutex<TimingTable> = Mutex::new(TimingTable::default());
}

/// Identifies the unit of work a sample belongs to - in the engine, one navigation.
///
/// Without this the table is one process-global bucket, so a second tab's numbers land
/// in the first tab's stats and nothing can be attributed to the page that caused it.
#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub struct ScopeId(pub uuid::Uuid);

thread_local! {
    /// Scope that samples recorded on this thread belong to.
    ///
    /// Thread-local, which is only sound because every span this crate records is taken
    /// around *synchronous* work: the render pipeline is a plain fn, parsing is a plain
    /// fn, and the rasterizer's spans are on the calling thread, outside its rayon
    /// `par_iter`. A scope must therefore never be held across an `.await` - the task
    /// could resume on another thread and samples would be attributed to whatever that
    /// thread was doing. Enter one around the synchronous unit instead.
    static CURRENT_SCOPE: std::cell::Cell<Option<ScopeId>> = const { std::cell::Cell::new(None) };
}

/// Attribute samples recorded on this thread to `scope` until the guard drops.
///
/// Nests: the previous scope is restored, not cleared.
#[must_use = "the scope ends as soon as the guard is dropped"]
pub fn enter_scope(scope: ScopeId) -> ScopeGuard {
    let previous = CURRENT_SCOPE.with(|c| c.replace(Some(scope)));
    ScopeGuard { previous }
}

/// Restores the enclosing scope when dropped. See [`enter_scope`].
pub struct ScopeGuard {
    previous: Option<ScopeId>,
}

impl Drop for ScopeGuard {
    fn drop(&mut self) {
        CURRENT_SCOPE.with(|c| c.set(self.previous));
    }
}

/// The scope samples on this thread are currently attributed to, if any.
#[must_use]
pub fn current_scope() -> Option<ScopeId> {
    CURRENT_SCOPE.with(std::cell::Cell::get)
}

/// Statistics for one namespace within `scope` only.
#[must_use]
pub fn snapshot_stats_for(scope: ScopeId) -> Vec<NamespaceStats> {
    TIMING_TABLE.lock().namespace_stats_for(Some(scope))
}

/// Drop everything recorded for `scope`.
///
/// The table otherwise grows for the life of the process, so a long browsing session
/// accumulates every navigation it ever ran. Call this when a navigation's numbers have
/// been read and are no longer wanted.
pub fn clear_scope(scope: ScopeId) {
    TIMING_TABLE.lock().clear_scope(scope);
}

/// Start a timer on the global table. No-op returning a nil id when the `timing`
/// feature is off.
///
/// The macros route through here rather than touching `TIMING_TABLE` directly, so the
/// feature check lives in this crate. A `cfg` inside a `#[macro_export]` body would be
/// evaluated against the *calling* crate's features, which is not what we want.
#[cfg(feature = "timing")]
pub fn start(namespace: &str, context: Option<String>) -> TimerId {
    TIMING_TABLE.lock().start_timer(namespace, context)
}

/// Timing disabled: hand back a nil id and take no lock.
#[cfg(not(feature = "timing"))]
pub fn start(_namespace: &str, _context: Option<String>) -> TimerId {
    TimerId::nil()
}

/// Stop a timer previously started with [`start`].
#[cfg(feature = "timing")]
pub fn stop(timer_id: TimerId) {
    TIMING_TABLE.lock().stop_timer(timer_id);
}

/// Timing disabled: nothing to stop.
#[cfg(not(feature = "timing"))]
pub fn stop(_timer_id: TimerId) {}

/// File a duration measured elsewhere under `namespace`.
#[cfg(feature = "timing")]
pub fn record(namespace: &str, duration_us: u64, context: Option<String>) {
    TIMING_TABLE.lock().record(namespace, duration_us, context);
}

/// Timing disabled: drop the sample.
#[cfg(not(feature = "timing"))]
pub fn record(_namespace: &str, _duration_us: u64, _context: Option<String>) {}

/// File a duration against an explicitly named scope.
///
/// Use this from async code, for the same reason as [`mark_in`]: the thread-local scope
/// only holds inside a synchronous unit.
#[cfg(feature = "timing")]
pub fn record_in(scope: ScopeId, namespace: &str, duration_us: u64, context: Option<String>) {
    let mut table = TIMING_TABLE.lock();
    let mut timer = Timer::finished(context, duration_us);
    timer.scope = Some(scope);
    table.insert_finished(namespace, timer);
}

/// Timing disabled: drop the sample.
#[cfg(not(feature = "timing"))]
pub fn record_in(_scope: ScopeId, _namespace: &str, _duration_us: u64, _context: Option<String>) {}

/// Stamp the start of `scope` - the epoch its marks are measured from.
#[cfg(feature = "timing")]
pub fn begin_scope(scope: ScopeId) {
    TIMING_TABLE.lock().begin_scope(scope);
}

/// Timing disabled: no epoch to stamp.
#[cfg(not(feature = "timing"))]
pub fn begin_scope(_scope: ScopeId) {}

/// Record a mark under `namespace`, measured from the current scope's epoch.
///
/// No-op when no scope is entered on this thread, or when that scope was never begun.
#[cfg(feature = "timing")]
pub fn mark(namespace: &str, context: Option<String>) {
    if let Some(scope) = current_scope() {
        TIMING_TABLE.lock().mark_in(scope, namespace, context);
    }
}

/// Timing disabled: drop the mark.
#[cfg(not(feature = "timing"))]
pub fn mark(_namespace: &str, _context: Option<String>) {}

/// Record a mark against an explicitly named scope.
///
/// Use this from async code. [`mark`] reads the thread-local scope, which is only valid
/// inside a synchronous unit; a task that resumed on another thread would either see no
/// scope or, worse, someone else's.
#[cfg(feature = "timing")]
pub fn mark_in(scope: ScopeId, namespace: &str, context: Option<String>) {
    TIMING_TABLE.lock().mark_in(scope, namespace, context);
}

/// Timing disabled: drop the mark.
#[cfg(not(feature = "timing"))]
pub fn mark_in(_scope: ScopeId, _namespace: &str, _context: Option<String>) {}

/// Returns a snapshot of all namespace statistics from the global timing table.
pub fn snapshot_stats() -> Vec<NamespaceStats> {
    TIMING_TABLE.lock().namespace_stats()
}

/// Clears all recorded timings from the global timing table.
pub fn reset_stats() {
    TIMING_TABLE.lock().clear();
}

/// Print the full timing table (all namespaces, aggregated stats) to stdout, auto-scaling units.
/// When `details` is true, also prints each individual timer's duration and context.
pub fn dump(details: bool) {
    println!("\n=== Timing table (all values aggregated since start) ===");
    TIMING_TABLE.lock().print_timings(details, Scale::Auto);
    println!();
}

/// RAII timer guard - stops the timer when dropped, regardless of how the
/// enclosing scope exits (normal return, early return, `?`, panic).
///
/// Obtain one via [`timing_guard!`](crate::timing_guard) or [`TimerGuard::start`].
pub struct TimerGuard {
    id: TimerId,
}

impl TimerGuard {
    pub fn start(namespace: &str, context: &str) -> Self {
        Self {
            id: start(namespace, Some(context.to_string())),
        }
    }

    pub fn start_anon(namespace: &str) -> Self {
        Self {
            id: start(namespace, None),
        }
    }
}

impl Drop for TimerGuard {
    fn drop(&mut self) {
        stop(self.id);
    }
}

#[allow(clippy::crate_in_macro_def)]
#[macro_export]
macro_rules! timing_start {
    ($namespace:expr, $context:expr) => {{
        $crate::timing::start($namespace, Some($context.to_string()))
    }};

    ($namespace:expr) => {{
        $crate::timing::start($namespace, None)
    }};
}

#[allow(clippy::crate_in_macro_def)]
#[macro_export]
macro_rules! timing_stop {
    ($timer_id:expr) => {{
        $crate::timing::stop($timer_id);
    }};
}

/// Record a duration measured somewhere else - work this process did not clock itself,
/// such as a fetch whose elapsed time the net stack reports back to us.
///
/// ```rust,ignore
/// timing_record!("net.fetch.image", elapsed, url.as_str());
/// ```
#[allow(clippy::crate_in_macro_def)]
#[macro_export]
macro_rules! timing_record {
    ($namespace:expr, $duration:expr, $context:expr) => {{
        $crate::timing::record($namespace, $duration.as_micros() as u64, Some($context.to_string()))
    }};

    ($namespace:expr, $duration:expr) => {{
        $crate::timing::record($namespace, $duration.as_micros() as u64, None)
    }};
}

/// Start a scoped timer that stops automatically when the returned guard drops.
///
/// Use this instead of `timing_start!/timing_stop!` whenever the measured
/// block has multiple exit paths (early returns, `?`, etc.).
///
/// ```rust,ignore
/// let _t = timing_guard!("net.fetch", url.as_str());
/// // timer stops when `_t` goes out of scope, on any path
/// ```
#[allow(clippy::crate_in_macro_def)]
#[macro_export]
macro_rules! timing_guard {
    ($namespace:expr, $context:expr) => {
        $crate::timing::TimerGuard::start($namespace, $context)
    };
    ($namespace:expr) => {
        $crate::timing::TimerGuard::start_anon($namespace)
    };
}

#[allow(clippy::crate_in_macro_def)]
#[macro_export]
macro_rules! timing_display {
    () => {{
        $crate::timing::TIMING_TABLE.lock().print_timings(false, Scale::Auto);
    }};

    ($scale:expr) => {{
        $crate::timing::TIMING_TABLE.lock().print_timings(false, $scale);
    }};

    ($details:expr, $scale:expr) => {{
        $crate::timing::TIMING_TABLE.lock().print_timings($details, $scale);
    }};
}

#[derive(Debug, Clone)]
pub struct Timer {
    id: TimerId,
    context: Option<String>,
    /// Unit of work this sample belongs to, captured when the timer was created.
    scope: Option<ScopeId>,
    #[cfg(not(target_arch = "wasm32"))]
    start: Instant,
    #[cfg(target_arch = "wasm32")]
    start: f64,
    #[cfg(not(target_arch = "wasm32"))]
    end: Option<Instant>,
    #[cfg(target_arch = "wasm32")]
    end: Option<f64>,
    duration_us: u64,
}

impl Timer {
    #[must_use]
    pub fn new(context: Option<String>) -> Timer {
        #[cfg(not(target_arch = "wasm32"))]
        let start = { Instant::now() };

        #[cfg(target_arch = "wasm32")]
        let start = {
            window()
                .and_then(|w| w.performance())
                .map(|p| p.now())
                .unwrap_or(f64::NAN)
        };

        Timer {
            id: new_timer_id(),
            context,
            scope: current_scope(),
            start,
            end: None,
            duration_us: 0,
        }
    }

    /// A timer that is already finished, carrying a duration measured elsewhere.
    /// `start` and `end` are stamped at construction so `has_finished()` holds; only
    /// `duration_us` is meaningful.
    #[must_use]
    pub fn finished(context: Option<String>, duration_us: u64) -> Timer {
        #[cfg(not(target_arch = "wasm32"))]
        let now = Instant::now();

        #[cfg(target_arch = "wasm32")]
        let now = window()
            .and_then(|w| w.performance())
            .map(|p| p.now())
            .unwrap_or(f64::NAN);

        Timer {
            id: new_timer_id(),
            context,
            scope: current_scope(),
            start: now,
            end: Some(now),
            duration_us,
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn start(&mut self) {
        self.start = Instant::now();
    }

    #[cfg(target_arch = "wasm32")]
    pub fn start(&mut self) {
        self.start = window()
            .and_then(|w| w.performance())
            .map(|p| p.now())
            .unwrap_or(f64::NAN);
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn end(&mut self) {
        let now = Instant::now();
        self.duration_us = now.duration_since(self.start).as_micros() as u64;
        self.end = Some(now);
    }

    #[cfg(target_arch = "wasm32")]
    pub fn end(&mut self) {
        self.end = window().and_then(|w| w.performance()).map(|p| p.now());
        self.duration_us = self.end.map(|e| (e - self.start) * 1000.0).unwrap_or(f64::NAN) as u64;
    }

    /// Microseconds since this timer started, whether or not it has ended.
    ///
    /// `duration()` reports 0 until `end()` is called; an epoch timer is never ended, so
    /// marks need this instead.
    #[must_use]
    #[cfg(not(target_arch = "wasm32"))]
    pub fn elapsed_us(&self) -> u64 {
        Instant::now().duration_since(self.start).as_micros() as u64
    }

    /// Microseconds since this timer started, whether or not it has ended.
    #[must_use]
    #[cfg(target_arch = "wasm32")]
    pub fn elapsed_us(&self) -> u64 {
        window()
            .and_then(|w| w.performance())
            .map(|p| ((p.now() - self.start) * 1000.0) as u64)
            .unwrap_or(0)
    }

    pub(crate) fn has_finished(&self) -> bool {
        self.end.is_some()
    }

    #[must_use]
    pub fn duration(&self) -> u64 {
        if self.end.is_some() {
            self.duration_us
        } else {
            0
        }
    }
}

#[cfg(test)]
mod scope_tests {
    use super::*;

    fn scope() -> ScopeId {
        ScopeId(uuid::Uuid::new_v4())
    }

    fn count_in(table: &TimingTable, sc: Option<ScopeId>, ns: &str) -> u64 {
        table
            .namespace_stats_for(sc)
            .into_iter()
            .find(|s| s.namespace == ns)
            .map_or(0, |s| s.count)
    }

    /// Samples land in the scope that was current when they were recorded, and a query for
    /// one scope must not see another's - the whole point of the change.
    #[test]
    fn samples_are_isolated_per_scope() {
        let (a, b) = (scope(), scope());
        let mut table = TimingTable::new();

        {
            let _g = enter_scope(a);
            table.record("decode.html", 100, None);
        }
        {
            let _g = enter_scope(b);
            table.record("decode.html", 200, None);
            table.record("decode.html", 300, None);
        }

        assert_eq!(count_in(&table, Some(a), "decode.html"), 1);
        assert_eq!(count_in(&table, Some(b), "decode.html"), 2);
        // The global view still aggregates everything.
        assert_eq!(count_in(&table, None, "decode.html"), 3);
    }

    /// Dropping the guard restores the enclosing scope rather than clearing it, so nesting
    /// a scope inside another does not silently unscope the outer work.
    #[test]
    fn scopes_nest_and_restore() {
        let (outer, inner) = (scope(), scope());
        assert_eq!(current_scope(), None);

        let _o = enter_scope(outer);
        assert_eq!(current_scope(), Some(outer));
        {
            let _i = enter_scope(inner);
            assert_eq!(current_scope(), Some(inner));
        }
        assert_eq!(current_scope(), Some(outer));
    }

    /// Work recorded with no scope entered stays unattributed rather than being folded
    /// into some other navigation.
    #[test]
    fn unscoped_samples_belong_to_no_scope() {
        let a = scope();
        let mut table = TimingTable::new();
        table.record("pipeline.total", 50, None);

        assert_eq!(count_in(&table, Some(a), "pipeline.total"), 0);
        assert_eq!(count_in(&table, None, "pipeline.total"), 1);
    }

    /// Clearing one scope must leave the others intact - this is what stops the table
    /// growing for the life of the process.
    #[test]
    fn clear_scope_drops_only_that_scope() {
        let (a, b) = (scope(), scope());
        let mut table = TimingTable::new();

        {
            let _g = enter_scope(a);
            table.record("decode.html", 100, None);
        }
        {
            let _g = enter_scope(b);
            table.record("decode.html", 200, None);
        }

        table.clear_scope(a);

        assert_eq!(count_in(&table, Some(a), "decode.html"), 0);
        assert_eq!(count_in(&table, Some(b), "decode.html"), 1);
        assert_eq!(count_in(&table, None, "decode.html"), 1);
    }

    /// Marks land in the scope's namespace and are measured from its epoch, so they are
    /// ordered the way the navigation actually happened.
    #[test]
    fn marks_are_measured_from_the_scope_epoch() {
        let a = scope();
        let mut table = TimingTable::new();
        table.begin_scope(a);

        table.mark_in(a, "page.dom_complete", None);
        std::thread::sleep(std::time::Duration::from_millis(5));
        table.mark_in(a, "page.load_complete", None);

        let at = |ns: &str| {
            table
                .namespace_stats_for(Some(a))
                .into_iter()
                .find(|s| s.namespace == ns)
                .map(|s| s.total_us)
                .expect("mark recorded")
        };

        // Later marks sit further from the epoch. That ordering is the whole point: a mark
        // answers "when", not "how long".
        assert!(
            at("page.load_complete") > at("page.dom_complete"),
            "load_complete ({}) should be later than dom_complete ({})",
            at("page.load_complete"),
            at("page.dom_complete")
        );
        assert!(at("page.load_complete") >= 5_000, "at least the 5ms we slept");
    }

    /// A mark against a scope that was never begun is dropped rather than recorded from a
    /// made-up epoch - a wrong number here would read as a real measurement.
    #[test]
    fn marks_without_an_epoch_are_dropped() {
        let a = scope();
        let mut table = TimingTable::new();

        table.mark_in(a, "page.first_paint", None);

        assert!(table.namespace_stats_for(Some(a)).is_empty());
    }

    /// Re-beginning a scope resets its epoch, so a reload measures from the reload.
    #[test]
    fn beginning_a_scope_again_resets_the_epoch() {
        let a = scope();
        let mut table = TimingTable::new();

        table.begin_scope(a);
        std::thread::sleep(std::time::Duration::from_millis(10));
        table.begin_scope(a);
        table.mark_in(a, "page.first_paint", None);

        let first_paint = table
            .namespace_stats_for(Some(a))
            .into_iter()
            .find(|s| s.namespace == "page.first_paint")
            .map(|s| s.total_us)
            .expect("mark recorded");

        assert!(
            first_paint < 10_000,
            "measured {first_paint}us - should be from the second begin_scope, not the first"
        );
    }

    /// Clearing a scope drops its epoch too, so a later mark cannot resurrect it.
    #[test]
    fn clearing_a_scope_drops_its_epoch() {
        let a = scope();
        let mut table = TimingTable::new();
        table.begin_scope(a);
        table.clear_scope(a);

        table.mark_in(a, "page.first_paint", None);

        assert!(table.namespace_stats_for(Some(a)).is_empty());
    }

    /// A namespace left with no samples after a clear disappears instead of lingering as
    /// an empty row.
    #[test]
    fn emptied_namespaces_are_removed() {
        let a = scope();
        let mut table = TimingTable::new();
        {
            let _g = enter_scope(a);
            table.record("net.fetch.css", 100, None);
        }
        table.clear_scope(a);
        assert!(table.namespace_stats_for(None).is_empty());
    }
}

#[cfg(test)]
mod tests {
    use rand::random;
    #[cfg(not(target_arch = "wasm32"))]
    use std::thread::sleep;
    use std::time::Duration;

    #[cfg(target_arch = "wasm32")]
    use {
        js_sys::wasm_bindgen::closure::Closure, std::sync::atomic::AtomicBool, std::sync::Arc,
        wasm_bindgen_test::wasm_bindgen_test_configure, wasm_bindgen_test::*, web_sys::wasm_bindgen::JsCast,
    };

    use super::*;

    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn test_timing_defaults() {
        let t = timing_start!("dns.lookup", "www.foo.bar");
        sleep(Duration::from_millis(10));
        timing_stop!(t);

        for _i in 0..10 {
            let t = timing_start!("html5.parse", "index.html");
            sleep(Duration::from_millis(random::<u64>() % 50));
            timing_stop!(t);
        }

        let t = timing_start!("html5.parse", "index.html");
        sleep(Duration::from_millis(20));
        timing_stop!(t);

        let t = timing_start!("html5.parse", "page2.html");
        sleep(Duration::from_millis(20));
        timing_stop!(t);

        let t = timing_start!("html5.parse", "page3.html");
        sleep(Duration::from_millis(20));
        timing_stop!(t);

        let t = timing_start!("css.parse");
        sleep(Duration::from_millis(20));
        timing_stop!(t);

        TIMING_TABLE.lock().print_timings(true, Scale::Auto);
    }

    #[wasm_bindgen_test]
    #[cfg(target_arch = "wasm32")]
    fn test_timing_defaults_wasm() {
        let window = &window().expect("no global `window` exists");

        let t = timing_start!("dns.lookup", "www.foo.bar");
        sleep(window, Duration::from_millis(10));
        timing_stop!(t);

        for _i in 0..10 {
            let t = timing_start!("html5.parse", "index.html");
            sleep(window, Duration::from_millis(random::<u64>() % 50));
            timing_stop!(t);
        }

        let t = timing_start!("html5.parse", "index.html");
        sleep(window, Duration::from_millis(20));
        timing_stop!(t);

        let t = timing_start!("html5.parse", "page2.html");
        sleep(window, Duration::from_millis(20));
        timing_stop!(t);

        let t = timing_start!("html5.parse", "page3.html");
        sleep(window, Duration::from_millis(20));
        timing_stop!(t);

        let t = timing_start!("css.parse");
        sleep(window, Duration::from_millis(20));
        timing_stop!(t);

        TIMING_TABLE.lock().print_timings(true, Scale::Auto);
    }

    //This should only be used for testing purposes
    #[cfg(target_arch = "wasm32")]
    fn sleep(window: &web_sys::Window, duration: Duration) {
        let finished = Arc::new(AtomicBool::new(false));
        let mut remaining_loops = 50_000 * duration.as_millis(); //just meant as a backup to avoid infinite loops

        let barrier = Arc::clone(&finished);
        let closure: Box<dyn Fn() -> ()> = Box::new(move || {
            barrier.store(true, std::sync::atomic::Ordering::SeqCst);
        });
        window
            .set_timeout_with_callback_and_timeout_and_arguments_0(
                Closure::wrap(closure).as_ref().unchecked_ref(),
                duration.as_millis() as i32,
            )
            .unwrap();

        while !finished.load(std::sync::atomic::Ordering::SeqCst) {
            std::hint::spin_loop();
            if remaining_loops == 0 {
                break;
            }
            remaining_loops -= 1;
        }
    }
}
