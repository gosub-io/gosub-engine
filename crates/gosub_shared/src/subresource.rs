//! Handing a preloaded subresource to whoever ends up needing it.
//!
//! The resource pipeline discovers a page's stylesheets, images and fonts while the HTML is
//! still parsing and fetches them straight away, which is what gets connections open early.
//! The code that actually *uses* those bytes runs later and elsewhere -- the CSS parser, the
//! media store, the font loader -- in places with no access to the async fetch stack, so each
//! one fetched the same URL again over its own blocking client. Every subresource on every
//! page was transferred twice.
//!
//! This is the hand-off between them. The pipeline announces a fetch before it starts and
//! deposits the bytes when it finishes; a consumer asks for a URL and gets those bytes,
//! waiting if the fetch is still in flight.
//!
//! Global, like the timing table next door, and for the same reason: the producer and the
//! consumers are in three different crates with no shared object between them.

use parking_lot::{Condvar, Mutex};
use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::Duration;

/// A fetched body and the content type it came with.
type Payload = (Option<String>, Vec<u8>);

enum Entry {
    /// A fetch is running. A consumer that asks now waits for it.
    InFlight,
    /// Bytes waiting to be claimed.
    Ready(Payload),
    /// The fetch finished without usable bytes. Recorded rather than forgotten so a waiter
    /// stops waiting immediately instead of sitting out its timeout.
    Failed,
}

#[derive(Default)]
struct Store {
    entries: HashMap<String, Entry>,
    /// Bytes held in `Ready` entries, so a page of large images cannot grow this without
    /// limit when nothing claims them.
    held: usize,
}

/// Roughly a page's worth of subresources. Past this, new arrivals are dropped rather than
/// stored: the consumer then fetches for itself, which is what it did before this existed.
const BUDGET: usize = 32 * 1024 * 1024;

/// How long a consumer waits for an in-flight fetch before giving up and doing its own.
///
/// A bound rather than an indefinite wait: if the pipeline is wedged, a slow page is a much
/// better failure than a hung one, and falling back is exactly the old behaviour.
const WAIT: Duration = Duration::from_secs(5);

fn store() -> &'static (Mutex<Store>, Condvar) {
    static STORE: OnceLock<(Mutex<Store>, Condvar)> = OnceLock::new();
    STORE.get_or_init(|| (Mutex::new(Store::default()), Condvar::new()))
}

/// Announce that a fetch for `url` has started, so a consumer asking for it waits rather
/// than starting a second one.
pub fn begin(url: &str) {
    let _ = claim(url);
}

/// Announce a fetch and say whether this caller is the one that has to make it.
///
/// `true` means nothing was in flight and nothing has been delivered, so the caller must
/// fetch. `false` means someone else already has it in hand -- a consumer that discovers a
/// resource the document scan already saw asks for it exactly this way, and gets told not to
/// fetch it twice.
pub fn claim(url: &str) -> bool {
    let (lock, _) = store();
    let mut s = lock.lock();
    // An existing entry is a fetch already announced or already delivered; leave it be.
    !s.entries.contains_key(url) && {
        s.entries.insert(url.to_string(), Entry::InFlight);
        true
    }
}

/// Deposit the bytes a fetch produced, waking anyone waiting for them.
pub fn complete(url: &str, content_type: Option<String>, body: Vec<u8>) {
    let (lock, cv) = store();
    let mut s = lock.lock();
    if s.held + body.len() > BUDGET {
        // Over budget: record the failure so waiters stop waiting and fetch for themselves.
        s.entries.insert(url.to_string(), Entry::Failed);
    } else {
        s.held += body.len();
        s.entries.insert(url.to_string(), Entry::Ready((content_type, body)));
    }
    cv.notify_all();
}

/// Record that a fetch produced nothing usable, so waiters stop waiting.
pub fn abandon(url: &str) {
    let (lock, cv) = store();
    let mut s = lock.lock();
    s.entries.insert(url.to_string(), Entry::Failed);
    cv.notify_all();
}

/// Claim the bytes for `url`, waiting up to [`WAIT`] if a fetch is still running.
///
/// `None` means "fetch it yourself": nothing was preloaded, the preload failed, or it took
/// too long. The bytes are removed on claim -- one consumer per URL is the norm, and holding
/// them for a second that will never come is what a budget exists to prevent.
pub fn take(url: &str) -> Option<Payload> {
    let (lock, cv) = store();
    let mut s = lock.lock();

    loop {
        match s.entries.get(url) {
            Some(Entry::Ready(_)) => {
                let Some(Entry::Ready(payload)) = s.entries.remove(url) else {
                    return None;
                };
                s.held = s.held.saturating_sub(payload.1.len());
                return Some(payload);
            }
            Some(Entry::Failed) => {
                s.entries.remove(url);
                return None;
            }
            // Nothing announced: the consumer is ahead of the pipeline, or this URL was
            // never preloaded at all. Either way it fetches for itself.
            None => return None,
            Some(Entry::InFlight) => {
                if cv.wait_for(&mut s, WAIT).timed_out() {
                    return None;
                }
            }
        }
    }
}

/// Drop everything. Called when a navigation commits: the previous page's leftovers are of
/// no use to the next one, and holding them is a slow leak across a long session.
pub fn clear() {
    let (lock, _) = store();
    let mut s = lock.lock();
    s.entries.clear();
    s.held = 0;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The store is global, so these run one at a time: `clear()` in one test would
    /// otherwise wipe an entry another is waiting on, which shows up as a flake rather than
    /// as a failure anyone can read.
    fn exclusively<T>(body: impl FnOnce() -> T) -> T {
        static LOCK: Mutex<()> = Mutex::new(());
        let _guard = LOCK.lock();
        clear();
        body()
    }

    #[test]
    fn bytes_survive_from_the_fetch_to_the_consumer() {
        exclusively(|| {
            begin("https://example.test/a.css");
            complete(
                "https://example.test/a.css",
                Some("text/css".into()),
                b"body{}".to_vec(),
            );

            let (kind, body) = take("https://example.test/a.css").expect("claimed");
            assert_eq!(kind.as_deref(), Some("text/css"));
            assert_eq!(body, b"body{}");
        });
    }

    #[test]
    fn a_url_is_claimed_once() {
        exclusively(|| {
            begin("https://example.test/b.css");
            complete("https://example.test/b.css", None, b"x".to_vec());

            assert!(take("https://example.test/b.css").is_some());
            // The second asker fetches for itself rather than waiting on bytes that are gone.
            assert!(take("https://example.test/b.css").is_none());
        });
    }

    #[test]
    fn nothing_preloaded_means_fetch_it_yourself() {
        exclusively(|| {
            assert!(take("https://example.test/never-seen").is_none());
        });
    }

    #[test]
    fn a_failed_fetch_does_not_leave_a_consumer_waiting() {
        exclusively(|| {
            begin("https://example.test/gone.png");
            abandon("https://example.test/gone.png");

            let started = std::time::Instant::now();
            assert!(take("https://example.test/gone.png").is_none());
            assert!(started.elapsed() < WAIT, "returned without sitting out the timeout");
        });
    }

    #[test]
    fn a_consumer_waits_for_a_fetch_that_is_still_running() {
        exclusively(|| {
            let url = "https://example.test/slow.png";
            begin(url);

            let writer = std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(120));
                complete(url, Some("image/png".into()), b"PNG".to_vec());
            });

            // Blocks until the fetch lands rather than starting a second one.
            let claimed = take(url).expect("waited for the in-flight fetch");
            assert_eq!(claimed.1, b"PNG");
            writer.join().unwrap();
        });
    }

    #[test]
    fn an_oversized_body_is_dropped_rather_than_held() {
        exclusively(|| {
            let url = "https://example.test/huge.bin";
            begin(url);
            complete(url, None, vec![0u8; BUDGET + 1]);
            // Refused, so the consumer fetches it itself and the budget is not blown.
            assert!(take(url).is_none());
        });
    }
}
