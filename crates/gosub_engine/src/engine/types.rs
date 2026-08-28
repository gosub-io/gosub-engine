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
