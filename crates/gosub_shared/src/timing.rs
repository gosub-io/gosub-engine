use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;
use lazy_static::lazy_static;

type TimerId = uuid::Uuid;

fn new_timer_id() -> TimerId {
    uuid::Uuid::new_v4()
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
        self.namespaces.entry(namespace.to_string()).or_insert(Vec::new()).push(timer.id);

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
                if timer.has_finished() {
                    durations.push(timer.duration_ms);
                }
            }
        }

        durations.sort();
        let count = durations.len() as u64;
        let total = durations.iter().sum();
        let min = durations.first().unwrap_or(&0).clone();
        let max = durations.last().unwrap_or(&0).clone();
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

    pub fn print_timings(&self, show_details: bool) {
        println!("Namespace            |    Count |      Total |        Min |        Max |        Avg |        50% |        75% |        95% |        99%");
        println!("----------------------------------------------------------------------------------------------------------------------------------------");
        for (namespace, timers) in &self.namespaces {
            let stats = self.get_stats(timers);
            println!("{:20} | {:8} | {:8}ms | {:8}ms | {:8}ms | {:8}ms | {:8}ms | {:8}ms | {:8}ms | {:8}ms", namespace, stats.count, stats.total, stats.min, stats.max, stats.avg, stats.p50, stats.p75, stats.p95, stats.p99);

            if show_details {
                for timer_id in timers {
                    let timer = self.timers.get(timer_id).unwrap();
                    if timer.has_finished() {
                        println!("  {:18} | {:8} | {:8}ms", timer.context.clone().unwrap_or("".into()), 1, timer.duration_ms);
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

macro_rules! timing_start {
    ($namespace:expr, $context:expr) => {{
        TIMING_TABLE.lock().unwrap().start_timer($namespace, Some($context.to_string()))
    }};

    ($namespace:expr) => {{
        TIMING_TABLE.lock().unwrap().start_timer($namespace, None)
    }};
}

macro_rules! timing_stop {
    ($timer_id:expr) => {{
        TIMING_TABLE.lock().unwrap().stop_timer($timer_id);
    }};
}


#[derive(Debug, Clone)]
pub struct Timer {
    id: TimerId,
    context: Option<String>,
    start: Instant,
    end: Option<Instant>,
    duration_ms: u64,
}

impl Timer {
    pub fn new(context: Option<String>) -> Timer {
        Timer {
            id: new_timer_id(),
            context,
            start: Instant::now(),
            end: None,
            duration_ms: 0
        }
    }

    pub fn start(&mut self) {
        self.start = Instant::now();
    }

    pub fn end(&mut self) {
        self.end = Some(Instant::now());
        self.duration_ms = self.end.expect("").duration_since(self.start).as_millis() as u64;
    }

    pub(crate) fn has_finished(&self) -> bool {
        return self.end.is_some();
    }

    pub fn duration(&self) -> u64 {
        if let Some(end) = self.end {
            end.duration_since(self.start).as_millis() as u64
        } else {
            0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread::sleep;
    use rand::random;

    #[test]
    fn test_timing_defaults() {

        let t = timing_start!("dns.lookup", "www.foo.bar");
        sleep(std::time::Duration::from_millis(10));
        timing_stop!(t);

        for i in 0..50 {
            let t = timing_start!("html5.parse", "index.html");
            sleep(std::time::Duration::from_millis(
                random::<u64>() % 50
            ));
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

        TIMING_TABLE.lock().unwrap().print_timings(true);
    }
}
