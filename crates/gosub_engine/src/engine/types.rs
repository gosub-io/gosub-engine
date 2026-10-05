use crate::engine::events::IoCommand;
use crate::events::{EngineEvent, TabCommand};

pub use gosub_sonar::types::{PeekBuf, RequestId};
use std::fmt::Display;
use uuid::Uuid;

/// Navigation ID is the same for each complete load, including iframes, resources redirect etc
#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub struct NavigationId(pub Uuid);

impl NavigationId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// This navigation as the subresource hand-off keys its entries.
    ///
    /// The hand-off is process-wide and sits below the engine, so it cannot name a navigation
    /// and takes an opaque scope instead. A navigation is the right unit: two documents share
    /// a cookie jar without sharing a request context -- the same URL fetched from different
    /// pages can carry different `Referer` and different SameSite cookies, so one page's bytes
    /// are not an answer to another's request for it. Zones are covered by the same key, a
    /// navigation belonging to exactly one of them.
    pub fn as_scope(&self) -> gosub_shared::subresource::Scope {
        self.0.as_u128()
    }
}

impl Default for NavigationId {
    fn default() -> Self {
        Self::new()
    }
}

impl Display for NavigationId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

// Defined channels for communication
pub type EventChannel = tokio::sync::broadcast::Sender<EngineEvent>;
/// Sender for the high-volume resource stream - see [`ResourceUpdate`](crate::events::ResourceUpdate).
pub type ResourceChannel = tokio::sync::broadcast::Sender<crate::events::ResourceUpdate>;
pub type IoChannel = tokio::sync::mpsc::UnboundedSender<IoCommand>;
pub type TabChannel = tokio::sync::mpsc::Sender<TabCommand>;
