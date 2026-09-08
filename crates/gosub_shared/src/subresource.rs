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
use std::time::{Duration, Instant};

/// A fetched body and the content type it came with.
type Payload = (Option<String>, Vec<u8>);

/// The navigation a preload belongs to.
///
/// Bytes fetched for one page are not an answer to another page's request for the same URL:
/// two documents can share a cookie jar without sharing a request context, and the same URL
/// fetched from each can carry a different `Referer` and different SameSite cookies. The
/// store is process-wide, which makes this the only thing keeping them apart -- and it keeps
/// zones apart too, a navigation belonging to exactly one zone. Opaque here because the
/// engine owns the navigation type and this crate sits below it.
pub type Scope = u128;

/// What a stored entry is keyed by: the same URL in two navigations is two entries.
type Key = (Scope, String);

enum Entry {
    /// A fetch is running. A consumer that asks now waits for it.
    InFlight,
    /// Bytes waiting to be claimed, and when they arrived. The arrival time is what makes
    /// the oldest evictable when the budget runs out.
    Ready(Payload, Instant),
    /// The fetch finished without usable bytes. Recorded rather than forgotten so a waiter
    /// stops waiting immediately instead of sitting out its timeout.
    Failed,
}

#[derive(Default)]
struct Store {
    entries: HashMap<Key, Entry>,
    /// Bytes held in `Ready` entries, so a page of large images cannot grow this without
    /// limit when nothing claims them.
    held: usize,
}

impl Store {
    /// Forget an entry's byte accounting before it is replaced or removed.
    ///
    /// Every path that overwrites an entry has to come through here. `held` only ever fell
    /// in `take`, so a second `complete` for a URL, or an `abandon` over bytes already
    /// delivered, left those bytes counted against the budget for the life of the process.
    /// Enough of that and every later fetch is refused storage.
    fn release(&mut self, key: &Key) {
        if let Some(Entry::Ready(payload, _)) = self.entries.get(key) {
            self.held = self.held.saturating_sub(payload.1.len());
        }
    }

    /// Drop claimable bytes, oldest first, until `wanted` fits under the budget.
    ///
    /// Nothing here is owed to anyone: a `Ready` entry is a guess that someone will ask, and
    /// a consumer that finds its guess gone simply fetches for itself, which is what it did
    /// before this module existed. Dropping the oldest guess to make room for a fresher one
    /// is a better trade than refusing every arrival from here on.
    fn make_room(&mut self, wanted: usize) {
        while self.held + wanted > BUDGET {
            let oldest = self
                .entries
                .iter()
                .filter_map(|(key, entry)| match entry {
                    Entry::Ready(_, at) => Some((*at, key.clone())),
                    _ => None,
                })
                .min_by_key(|(at, _)| *at);

            let Some((_, key)) = oldest else {
                // Nothing claimable left to give up: the budget is spoken for by fetches
                // still running, and this arrival goes unstored.
                return;
            };
            self.release(&key);
            self.entries.remove(&key);
        }
    }
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
pub fn begin(scope: Scope, url: &str) {
    let _ = claim(scope, url);
}

/// Announce a fetch and say whether this caller is the one that has to make it.
///
/// `true` means nothing was in flight and nothing has been delivered, so the caller must
/// fetch. `false` means someone else already has it in hand -- a consumer that discovers a
/// resource the document scan already saw asks for it exactly this way, and gets told not to
/// fetch it twice.
pub fn claim(scope: Scope, url: &str) -> bool {
    let (lock, _) = store();
    let mut s = lock.lock();
    let key = (scope, url.to_string());
    // An existing entry is a fetch already announced or already delivered; leave it be.
    !s.entries.contains_key(&key) && {
        s.entries.insert(key, Entry::InFlight);
        true
    }
}

/// Deposit the bytes a fetch produced, waking anyone waiting for them.
pub fn complete(scope: Scope, url: &str, content_type: Option<String>, body: Vec<u8>) {
    let (lock, cv) = store();
    let mut s = lock.lock();
    let key = (scope, url.to_string());

    s.release(&key);
    s.make_room(body.len());
    if s.held + body.len() > BUDGET {
        // Still no room, so the bytes are dropped rather than stored. Recorded as a failure
        // so a waiter stops waiting and fetches for itself.
        s.entries.insert(key, Entry::Failed);
    } else {
        s.held += body.len();
        s.entries
            .insert(key, Entry::Ready((content_type, body), Instant::now()));
    }
    cv.notify_all();
}

/// Record that a fetch produced nothing usable, so waiters stop waiting.
pub fn abandon(scope: Scope, url: &str) {
    let (lock, cv) = store();
    let mut s = lock.lock();
    let key = (scope, url.to_string());
    s.release(&key);
    s.entries.insert(key, Entry::Failed);
    cv.notify_all();
}

/// Claim the bytes for `url`, waiting up to [`WAIT`] if a fetch is still running.
///
/// `None` means "fetch it yourself": nothing was preloaded, the preload failed, or it took
/// too long. The bytes are removed on claim -- one consumer per URL is the norm, and holding
/// them for a second that will never come is what a budget exists to prevent.
pub fn take(scope: Scope, url: &str) -> Option<Payload> {
    let (lock, cv) = store();
    let mut s = lock.lock();
    let key = (scope, url.to_string());
    // An absolute deadline, not a fresh `WAIT` per wakeup: `complete` and `abandon` wake
    // *every* waiter, so on a busy page a relative wait is restarted by other people's
    // fetches and this one can sit here far longer than the bound it is supposed to have.
    let deadline = Instant::now() + WAIT;

    loop {
        match s.entries.get(&key) {
            Some(Entry::Ready(..)) => {
                let Some(Entry::Ready(payload, _)) = s.entries.remove(&key) else {
                    return None;
                };
                s.held = s.held.saturating_sub(payload.1.len());
                return Some(payload);
            }
            Some(Entry::Failed) => {
                s.entries.remove(&key);
                return None;
            }
            // Nothing announced: the consumer is ahead of the pipeline, or this URL was
            // never preloaded at all. Either way it fetches for itself.
            None => return None,
            Some(Entry::InFlight) => {
                if cv.wait_until(&mut s, deadline).timed_out() {
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

    /// The navigation these tests fetch for. Which one does not matter, only that the test
    /// about isolation uses a different one.
    const ZONE: Scope = 1;

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
            begin(ZONE, "https://example.test/a.css");
            complete(
                ZONE,
                "https://example.test/a.css",
                Some("text/css".into()),
                b"body{}".to_vec(),
            );

            let (kind, body) = take(ZONE, "https://example.test/a.css").expect("claimed");
            assert_eq!(kind.as_deref(), Some("text/css"));
            assert_eq!(body, b"body{}");
        });
    }

    #[test]
    fn a_url_is_claimed_once() {
        exclusively(|| {
            begin(ZONE, "https://example.test/b.css");
            complete(ZONE, "https://example.test/b.css", None, b"x".to_vec());

            assert!(take(ZONE, "https://example.test/b.css").is_some());
            // The second asker fetches for itself rather than waiting on bytes that are gone.
            assert!(take(ZONE, "https://example.test/b.css").is_none());
        });
    }

    #[test]
    fn nothing_preloaded_means_fetch_it_yourself() {
        exclusively(|| {
            assert!(take(ZONE, "https://example.test/never-seen").is_none());
        });
    }

    #[test]
    fn a_failed_fetch_does_not_leave_a_consumer_waiting() {
        exclusively(|| {
            begin(ZONE, "https://example.test/gone.png");
            abandon(ZONE, "https://example.test/gone.png");

            let started = std::time::Instant::now();
            assert!(take(ZONE, "https://example.test/gone.png").is_none());
            assert!(started.elapsed() < WAIT, "returned without sitting out the timeout");
        });
    }

    #[test]
    fn a_consumer_waits_for_a_fetch_that_is_still_running() {
        exclusively(|| {
            let url = "https://example.test/slow.png";
            begin(ZONE, url);

            let writer = std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(120));
                complete(ZONE, url, Some("image/png".into()), b"PNG".to_vec());
            });

            // Blocks until the fetch lands rather than starting a second one.
            let claimed = take(ZONE, url).expect("waited for the in-flight fetch");
            assert_eq!(claimed.1, b"PNG");
            writer.join().unwrap();
        });
    }

    #[test]
    fn an_oversized_body_is_dropped_rather_than_held() {
        exclusively(|| {
            let url = "https://example.test/huge.bin";
            begin(ZONE, url);
            complete(ZONE, url, None, vec![0u8; BUDGET + 1]);
            // Refused, so the consumer fetches it itself and the budget is not blown.
            assert!(take(ZONE, url).is_none());
        });
    }

    /// Two pages do not share a request context even when they share a cookie jar: the same
    /// URL fetched from each can carry a different `Referer` and different SameSite cookies,
    /// so one page's bytes are not an answer to the other's request for it. Zones fall out of
    /// the same key, a navigation belonging to exactly one of them.
    #[test]
    fn one_page_does_not_answer_another_pages_request() {
        exclusively(|| {
            const OTHER: Scope = 2;
            let url = "https://example.test/private.json";

            begin(ZONE, url);
            complete(ZONE, url, None, b"secret".to_vec());

            // The other page sees nothing announced and fetches for itself.
            assert!(take(OTHER, url).is_none());
            // And the bytes are still there for the page they were fetched for.
            assert_eq!(take(ZONE, url).expect("still claimable").1, b"secret");
        });
    }

    /// `held` is what the budget is enforced against, so anything that replaces stored bytes
    /// has to give their accounting back. It only ever fell in `take`, which meant a second
    /// `complete`, or an `abandon` over delivered bytes, charged the budget forever.
    #[test]
    fn replacing_stored_bytes_gives_their_budget_back() {
        exclusively(|| {
            let url = "https://example.test/replaced.png";

            begin(ZONE, url);
            complete(ZONE, url, None, vec![0u8; 1024]);
            complete(ZONE, url, None, vec![0u8; 16]);
            abandon(ZONE, url);

            let (lock, _) = store();
            assert_eq!(lock.lock().held, 0, "nothing is stored, so nothing is charged");
        });
    }

    /// A full store gives up its oldest guess rather than refusing every arrival from then
    /// on. Nothing is owed to a `Ready` entry: whoever asks for one that is gone fetches for
    /// itself, exactly as it would have without this module.
    #[test]
    fn a_full_store_evicts_the_oldest_rather_than_failing_everything() {
        exclusively(|| {
            let half = BUDGET / 2 + 1;
            begin(ZONE, "https://example.test/old.bin");
            complete(ZONE, "https://example.test/old.bin", None, vec![0u8; half]);
            // Far enough apart to order, without making the test slow.
            std::thread::sleep(Duration::from_millis(5));
            begin(ZONE, "https://example.test/new.bin");
            complete(ZONE, "https://example.test/new.bin", None, vec![0u8; half]);

            // The newcomer is stored, and the entry it displaced is simply not there.
            assert!(take(ZONE, "https://example.test/new.bin").is_some());
            assert!(take(ZONE, "https://example.test/old.bin").is_none());
        });
    }
}
