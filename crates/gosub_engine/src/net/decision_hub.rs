//! Engine-side decision plumbing.
//!
//! When a top-level resource has loaded far enough for the UA to decide what to do with it
//! (render, download, open externally, ...), the engine parks the fetch on a
//! [`DecisionHub`] waiter and emits a `NavigationEvent::DecisionRequired` carrying a
//! [`DecisionToken`]. The UA answers via the engine API, which resolves the waiter with
//! the chosen [`Action`](crate::engine::types::Action).
//!
//! This used to live in the (now removed) `gosub_net` crate; the generic fetching layer
//! moved to the external `gosub-sonar` crate, which has no notion of UA decisions, so the
//! decision machinery is implemented here.

use crate::engine::types::Action;
use tokio::sync::oneshot;
use uuid::Uuid;

/// Correlation handle for a pending decision (stable across the decision lifecycle)
#[derive(Clone, Copy, Debug, Hash, Eq, PartialEq)]
pub struct DecisionToken(pub Uuid);

impl Default for DecisionToken {
    fn default() -> Self {
        Self::new()
    }
}

impl DecisionToken {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

pub struct DecisionHub {
    waiters: dashmap::DashMap<DecisionToken, oneshot::Sender<Action>>,
}

impl Default for DecisionHub {
    fn default() -> Self {
        Self::new()
    }
}

impl DecisionHub {
    pub fn new() -> Self {
        Self {
            waiters: dashmap::DashMap::new(),
        }
    }

    #[allow(unused)]
    pub fn register(&self) -> (DecisionToken, oneshot::Receiver<Action>) {
        let token = DecisionToken::new();
        let (tx, rx) = oneshot::channel();
        self.waiters.insert(token, tx);
        (token, rx)
    }

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

    fn sample_action() -> Action {
        Action::Cancel
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
        hub.fulfill(DecisionToken(Uuid::new_v4()), sample_action());
        assert!(hub.waiters.is_empty());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn fulfill_is_at_most_once() {
        let hub = DecisionHub::new();
        let (token, rx) = hub.register();
        hub.fulfill(token, sample_action());
        hub.fulfill(token, sample_action());
        let decided = timeout(Duration::from_millis(100), rx)
            .await
            .expect("timed out")
            .expect("sender dropped");
        assert_eq!(format!("{:?}", decided), format!("{:?}", sample_action()));
        assert!(hub.waiters.get(&token).is_none());
    }
}
