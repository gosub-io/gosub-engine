use crate::types::{Action, DecisionToken};
use tokio::sync::oneshot;

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
