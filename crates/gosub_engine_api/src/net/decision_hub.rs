//! Sometimes, the decision manager from the engine cannot decide on itself what to do
//! with a response, and needs to ask the user-agent (UA) for input.
//! This is common for navigation responses, where the UA may want to ask the user
//! whether to render, download, or cancel a response with an unknown or suspicious
//! MIME type.
//!
//! `DecisionHub` lets a producer register a **DecisionToken** and wait on a
//! `oneshot::Receiver<Action>`, while some other task (often on another thread)
//! later **fulfills** that token with the chosen `Action`.
//!
//! - IO/fetcher side calls [`DecisionHub::register`], gets `(token, rx)` and
//!   emits a `NavigationResponse` to the UA containing `token`.
//! - UA/tab decides action, then calls [`DecisionHub::fulfill`] with the same `token`.
//! - The IO/fetcher awaits `rx.await` and proceeds accordingly.

use crate::Action;
use tokio::sync::oneshot;

/// Correlation handle for a pending decision.
#[derive(Clone, Copy, Debug, Hash, Eq, PartialEq)]
pub struct DecisionToken(uuid::Uuid);

impl Default for DecisionToken {
    fn default() -> Self {
        Self::new()
    }
}

impl DecisionToken {
    /// Create a new unique token.
    #[inline]
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4())
    }
}

/// Rendezvous hub mapping `DecisionToken` to `oneshot::Sender<Action>`.
pub struct DecisionHub {
    /// Map of active waiters keyed by DecisionToken.
    waiters: dashmap::DashMap<DecisionToken, oneshot::Sender<Action>>,
}

impl DecisionHub {
    // Create a new DecisionHub
    pub fn new() -> Self {
        Self {
            waiters: dashmap::DashMap::new(),
        }
    }

    /// Register a new waiter and obtain its `(DecisionToken, oneshot::Receiver<Action>)`.
    ///
    /// Call this on the side that needs to **await** the user-agent/UA decision
    /// (typically the IO/fetcher task right after it produced `ResponseTop`).
    ///
    /// The returned `DecisionToken` should be propagated to the UA so it can call
    /// [`fulfill`](Self::fulfill) with the final `Action`.
    #[inline]
    #[allow(unused)]
    pub fn register(&self) -> (DecisionToken, oneshot::Receiver<Action>) {
        let token = DecisionToken::new();
        let (tx, rx) = oneshot::channel();
        self.waiters.insert(token, tx);
        (token, rx)
    }

    /// Deliver the decision for `token`, waking the waiter if it still exists.
    ///
    /// This is **idempotent**: if there is no current waiter (unknown token,
    /// already fulfilled, or receiver dropped), this is a no-op.
    #[inline]
    pub fn fulfill(&self, token: DecisionToken, action: Action) {
        if let Some((_, tx)) = self.waiters.remove(&token) {
            let _ = tx.send(action);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use tokio::time::{timeout, Duration};
    use uuid::Uuid;

    // Pick a variant that certainly exists in your codebase.
    // From our prior discussions you have variants like Render/Download/Cancel/etc.
    // If your enum differs, just switch these to a variant you have.
    fn sample_action() -> Action {
        // safest bet across your project is likely `Action::Cancel`
        // adjust if needed:
        #[allow(unreachable_code, clippy::diverging_sub_expression)]
        {
            // Replace with your actual constructor/variant if different.
            #[allow(dead_code)]
            enum _Check {}
            Action::Cancel
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn register_returns_unique_tokens() {
        let hub = DecisionHub::new();
        let mut set = HashSet::new();
        for _ in 0..10_000 {
            let (t, _rx) = hub.register();
            assert!(set.insert(t), "duplicate token: {:?}", t);
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn fulfill_delivers_action() {
        let hub = DecisionHub::new();
        let (token, rx) = hub.register();

        // fulfill from "another thread"
        hub.fulfill(token, sample_action());

        let decided = timeout(Duration::from_millis(100), rx)
            .await
            .expect("timed out")
            .expect("sender dropped");
        assert_eq!(format!("{:?}", decided), format!("{:?}", sample_action()));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn fulfill_unknown_token_is_noop() {
        let hub = DecisionHub::new();
        // Unknown token (never registered)
        hub.fulfill(DecisionToken(Uuid::new_v4()), sample_action());
        // Nothing to assert other than "no panic" and "no entry got created".
        assert!(hub.waiters.is_empty());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn fulfill_is_at_most_once() {
        let hub = DecisionHub::new();
        let (token, rx) = hub.register();

        hub.fulfill(token, sample_action());
        hub.fulfill(token, sample_action()); // second call is ignored

        let decided = timeout(Duration::from_millis(100), rx)
            .await
            .expect("timed out")
            .expect("sender dropped");
        assert_eq!(format!("{:?}", decided), format!("{:?}", sample_action()));

        // Hub should have removed the waiter on first fulfill.
        assert!(hub.waiters.get(&token).is_none());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn dropping_receiver_before_fulfill_is_ok() {
        let hub = DecisionHub::new();
        let (token, rx) = hub.register();
        drop(rx); // receiver gone

        // This will attempt to send and get Err(action) internally; we ignore it.
        hub.fulfill(token, sample_action());

        // Entry must be removed even if receiver was dropped.
        assert!(hub.waiters.get(&token).is_none());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_register_and_fulfill() {
        use std::sync::Arc;

        let hub = Arc::new(DecisionHub::new());

        let n = 1_000usize;
        let mut wait_handles = Vec::with_capacity(n);

        for _ in 0..n {
            let (token, rx) = hub.register();

            // Spawn waiter (no need for hub here)
            let waiter = tokio::spawn(async move {
                let res = tokio::time::timeout(Duration::from_secs(2), rx).await;
                res.expect("waiter timed out").expect("sender dropped")
            });

            // Fulfill from a different task — move an Arc clone in
            let hub2 = Arc::clone(&hub);
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_micros(10)).await;
                hub2.fulfill(token, sample_action());
            });

            wait_handles.push(waiter);
        }

        for h in wait_handles {
            let decided = h.await.expect("waiter task panicked");
            assert_eq!(format!("{:?}", decided), format!("{:?}", sample_action()));
        }

        assert!(hub.waiters.is_empty());
    }
}
