use lazy_static::lazy_static;
use std::collections::HashMap;
use std::fmt::{Display, Formatter};
use std::sync::Mutex;
use std::time::Instant;

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
        self.namespaces
            .entry(namespace.to_string())
            .or_default()
            .push(timer.id);

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
            println!("{:20} | {:>8} | {:>10} | {:>10} | {:>10} | {:>10} | {:>10} | {:>10} | {:>10} | {:>10}", namespace,
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
        $crate::timing::TIMING_TABLE
            .lock()
            .unwrap()
            .stop_timer($timer_id);
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
    start: Instant,
    end: Option<Instant>,
    duration_us: u64,
}

impl Timer {
    pub fn new(context: Option<String>) -> Timer {
        Timer {
            id: new_timer_id(),
            context,
            start: Instant::now(),
            end: None,
            duration_us: 0,
        }
    }

    pub fn start(&mut self) {
        self.start = Instant::now();
    }

    pub fn end(&mut self) {
        self.end = Some(Instant::now());
        self.duration_us = self.end.expect("").duration_since(self.start).as_micros() as u64;
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
    use super::*;
    use rand::random;
    use std::thread::sleep;

    #[test]
    fn test_timing_defaults() {
        let t = timing_start!("dns.lookup", "www.foo.bar");
        sleep(std::time::Duration::from_millis(10));
        timing_stop!(t);

        for _i in 0..10 {
            let t = timing_start!("html5.parse", "index.html");
            sleep(std::time::Duration::from_millis(random::<u64>() % 50));
            timing_stop!(t);
        }

        let t = timing_start!("html5.parse", "index.html");
        sleep(std::time::Duration::from_millis(20));
        timing_stop!(t);

        let t = timing_start!("html5.parse", "page2.html");
        sleep(std::time::Duration::from_millis(20));
        timing_stop!(t);

        let t = timing_start!("html5.parse", "page3.html");
        sleep(std::time::Duration::from_millis(20));
        timing_stop!(t);

        let t = timing_start!("css.parse");
        sleep(std::time::Duration::from_millis(20));
        timing_stop!(t);

        TIMING_TABLE
            .lock()
            .unwrap()
            .print_timings(true, Scale::Auto);
    }
}
