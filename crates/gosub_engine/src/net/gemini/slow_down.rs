//! Backing off from a server that answered `44`, slow down.
//!
//! The specification has the client back off exponentially: the delay between its requests
//! to the server doubles until the server stops answering `44`. Each host and port has its
//! delay here, starting at [`FIRST_DELAY`]. A request inside it waits it out, unless that is
//! longer than [`LONGEST_WAIT`]: such a request is not sent, so a navigation is never held
//! for minutes, and the page it gets says when the server is asked again.
//!
//! The delays live in the process that serves gemini, for all zones together: a zone can
//! see that another one was told to slow down by the same server, by waiting for it.

use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::LazyLock;
use std::time::{Duration, Instant};

/// The delay after a first `44`.
pub const FIRST_DELAY: Duration = Duration::from_secs(1);

/// The longest a request waits for its server's delay to run out before it is sent.
pub const LONGEST_WAIT: Duration = Duration::from_secs(10);

/// A ceiling no doubling goes past, so the arithmetic never overflows.
const CEILING: Duration = Duration::from_secs(24 * 60 * 60);

/// A host and port, as `known_hosts::key` makes them.
type Server = (String, u16);

static DELAYS: LazyLock<Mutex<Delays>> = LazyLock::new(Mutex::default);

/// How long a request to `server` has to wait before it may be sent, if at all.
pub fn waiting(server: &Server) -> Option<Duration> {
    DELAYS.lock().waiting(server, Instant::now())
}

/// Record how `server` answered: the delay it is now left alone for when it said slow down.
pub fn answered(server: &Server, slow_down: bool) -> Option<Duration> {
    DELAYS.lock().answered(server, slow_down, Instant::now())
}

/// Each server's delay, and when it runs out.
#[derive(Debug, Default)]
struct Delays(HashMap<Server, Backoff>);

#[derive(Debug, Clone, Copy)]
struct Backoff {
    delay: Duration,
    until: Instant,
}

impl Delays {
    fn waiting(&self, server: &Server, now: Instant) -> Option<Duration> {
        let wait = self.0.get(server)?.until.checked_duration_since(now)?;
        (!wait.is_zero()).then_some(wait)
    }

    /// A `44` doubles the delay, starting from [`FIRST_DELAY`]; any other answer ends it.
    fn answered(&mut self, server: &Server, slow_down: bool, now: Instant) -> Option<Duration> {
        if !slow_down {
            self.0.remove(server);
            return None;
        }
        let delay = match self.0.get(server) {
            Some(backoff) => (backoff.delay * 2).min(CEILING),
            None => FIRST_DELAY,
        };
        self.0.insert(
            server.clone(),
            Backoff {
                delay,
                until: now + delay,
            },
        );
        Some(delay)
    }
}

/// `delay` in words, rounded up to whole seconds.
pub fn in_words(delay: Duration) -> String {
    let secs = delay.as_secs() + u64::from(delay.subsec_nanos() > 0);
    match secs {
        1 => "1 second".into(),
        secs => format!("{secs} seconds"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server() -> Server {
        ("h.test".into(), 1965)
    }

    #[test]
    fn the_delay_doubles_until_the_server_stops_saying_slow_down() {
        let mut delays = Delays::default();
        let t0 = Instant::now();
        assert_eq!(delays.waiting(&server(), t0), None);
        assert_eq!(delays.answered(&server(), true, t0), Some(FIRST_DELAY));
        assert_eq!(delays.waiting(&server(), t0), Some(FIRST_DELAY));
        let t1 = t0 + Duration::from_millis(400);
        assert_eq!(delays.waiting(&server(), t1), Some(Duration::from_millis(600)));
        assert_eq!(delays.waiting(&server(), t0 + FIRST_DELAY), None);

        let t2 = t0 + Duration::from_secs(5);
        assert_eq!(delays.answered(&server(), true, t2), Some(Duration::from_secs(2)));
        assert_eq!(delays.answered(&server(), true, t2), Some(Duration::from_secs(4)));
        assert_eq!(delays.waiting(&server(), t2), Some(Duration::from_secs(4)));
        // Another server is not held back by it.
        assert_eq!(delays.waiting(&("h.test".into(), 1966), t2), None);

        assert_eq!(delays.answered(&server(), false, t2), None);
        assert_eq!(delays.waiting(&server(), t2), None);
        assert_eq!(delays.answered(&server(), true, t2), Some(FIRST_DELAY));
    }

    #[test]
    fn the_delay_stops_doubling_at_its_ceiling() {
        let mut delays = Delays::default();
        let now = Instant::now();
        for _ in 0..80 {
            delays.answered(&server(), true, now);
        }
        assert_eq!(delays.answered(&server(), true, now), Some(CEILING));
    }

    #[test]
    fn a_delay_is_said_in_whole_seconds() {
        assert_eq!(in_words(Duration::from_secs(1)), "1 second");
        assert_eq!(in_words(Duration::from_millis(1200)), "2 seconds");
        assert_eq!(in_words(Duration::from_secs(16)), "16 seconds");
    }
}
