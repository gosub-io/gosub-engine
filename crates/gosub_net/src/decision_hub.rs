//! `DecisionHub` lets a producer register a **DecisionToken** and wait on a
//! `oneshot::Receiver<Action>`, while some other task (often on another thread)
//! later **fulfills** that token with the chosen `Action`.

use crate::types::Action;
use tokio::sync::oneshot;

/// Correlation handle for a pending decision.
#[derive(Clone, Copy, Debug, Hash, Eq, PartialEq)]
pub struct DecisionToken(uuid::Uuid);

impl DecisionToken {
    /// Create a new unique token.
    #[inline]
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4())
    }
}

impl Default for DecisionToken {
    fn default() -> Self {
        Self::new()
    }
}

/// Rendezvous hub mapping `DecisionToken` to `oneshot::Sender<Action>`.
pub struct DecisionHub {
    waiters: dashmap::DashMap<DecisionToken, oneshot::Sender<Action>>,
}

impl DecisionHub {
    pub fn new() -> Self {
        Self {
            waiters: dashmap::DashMap::new(),
        }
    }

    /// Register a new waiter and obtain its `(DecisionToken, oneshot::Receiver<Action>)`.
    #[inline]
    #[allow(unused)]
    pub fn register(&self) -> (DecisionToken, oneshot::Receiver<Action>) {
        let token = DecisionToken::new();
        let (tx, rx) = oneshot::channel();
        self.waiters.insert(token, tx);
        (token, rx)
    }

    /// Deliver the decision for `token`, waking the waiter if it still exists.
    #[inline]
    pub fn fulfill(&self, token: DecisionToken, action: Action) {
        if let Some((_, tx)) = self.waiters.remove(&token) {
            let _ = tx.send(action);
        }
    }
}

impl Default for DecisionHub {
    fn default() -> Self {
        Self::new()
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

    #[tokio::test(flavor = "current_thread")]
    async fn dropping_receiver_before_fulfill_is_ok() {
        let hub = DecisionHub::new();
        let (token, rx) = hub.register();
        drop(rx);

        hub.fulfill(token, sample_action());

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

            let waiter = tokio::spawn(async move {
                let res = tokio::time::timeout(Duration::from_secs(2), rx).await;
                res.expect("waiter timed out").expect("sender dropped")
            });

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
