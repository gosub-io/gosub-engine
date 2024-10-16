use std::collections::HashMap;
use std::fmt::{Display, Formatter};
use std::sync::Mutex;
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

fn percentage_to_index(count: u64, percentage: f64) -> usize {
    (count as f64 * percentage) as usize
}

impl TimingTable {
    pub fn new() -> TimingTable {
        TimingTable {
            timers: HashMap::new(),
            namespaces: HashMap::new(),
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

        durations.sort();
        let count = durations.len() as u64;
        let total = durations.iter().sum();
        let min = *durations.first().unwrap_or(&0);
        let max = *durations.last().unwrap_or(&0);
        let avg = total / count;
        let p50 = durations[percentage_to_index(count, 0.50)];
        let p75 = durations[percentage_to_index(count, 0.75)];
        let p95 = durations[percentage_to_index(count, 0.95)];
        let p99 = durations[percentage_to_index(count, 0.99)];

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

    fn scale(&self, value: u64, scale: Scale) -> String {
        match scale {
            Scale::MicroSecond => format!("{}µs", value),
            Scale::MilliSecond => format!("{}ms", value / 1000),
            Scale::Second => format!("{}s", value / (1000 * 1000)),
            Scale::Auto => {
                if value < 1000 {
                    format!("{}µs", value)
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
                    let timer = self.timers.get(timer_id).unwrap();
                    if timer.has_finished() {
                        println!(
                            "                     | {:>8} | {:>10} | {}",
                            1,
                            self.scale(timer.duration_us, scale.clone()),
                            timer.context.clone().unwrap_or("".into())
                        );
                    }
                }
            }
        }
    }

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

#[allow(clippy::crate_in_macro_def)]
#[macro_export]
macro_rules! timing_start {
    ($namespace:expr, $context:expr) => {{
        $crate::timing::TIMING_TABLE
            .lock()
            .unwrap()
            .start_timer($namespace, Some($context.to_string()))
    }};

    ($namespace:expr) => {{
        $crate::timing::TIMING_TABLE
            .lock()
            .unwrap()
            .start_timer($namespace, None)
    }};
}

#[allow(clippy::crate_in_macro_def)]
#[macro_export]
macro_rules! timing_stop {
    ($timer_id:expr) => {{
        $crate::timing::TIMING_TABLE.lock().expect("Poisoned").stop_timer($timer_id);
    }};
}

#[allow(clippy::crate_in_macro_def)]
#[macro_export]
macro_rules! timing_display {
    () => {{
        $crate::timing::TIMING_TABLE
            .lock()
            .unwrap()
            .print_timings(false, Scale::Auto);
    }};

    ($scale:expr) => {{
        $crate::timing::TIMING_TABLE
            .lock()
            .unwrap()
            .print_timings(false, $scale);
    }};

    ($details:expr, $scale:expr) => {{
        $crate::timing::TIMING_TABLE
            .lock()
            .unwrap()
            .print_timings($details, $scale);
    }};
}

#[derive(Debug, Clone)]
pub struct Timer {
    id: TimerId,
    context: Option<String>,
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
            start,
            end: None,
            duration_us: 0,
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
        self.end = Some(Instant::now());
        self.duration_us = self.end.expect("").duration_since(self.start).as_micros() as u64;
    }

    #[cfg(target_arch = "wasm32")]
    pub fn end(&mut self) {
        self.end = window().and_then(|w| w.performance()).map(|p| p.now());
        self.duration_us = self.end.map(|e| (e - self.start) * 1000.0).unwrap_or(f64::NAN) as u64;
    }

    pub(crate) fn has_finished(&self) -> bool {
        self.end.is_some()
    }

    pub fn duration(&self) -> u64 {
        if self.end.is_some() {
            self.duration_us
        } else {
            0
        }
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

        TIMING_TABLE.lock().expect("Poisoned").print_timings(true, Scale::Auto);
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

        TIMING_TABLE.lock().expect("Poisoned").print_timings(true, Scale::Auto);
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
