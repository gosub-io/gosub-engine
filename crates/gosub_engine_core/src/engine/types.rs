use crate::events::{EngineEvent, TabCommand};

// Defined channels for communication
pub type EventChannel = tokio::sync::broadcast::Sender<EngineEvent>;
pub type TabChannel = tokio::sync::mpsc::Sender<TabCommand>;

// Re-export IoChannel from gosub_net
pub use gosub_net::io_types::IoChannel;

// Re-export shared types from gosub_net
pub use gosub_net::types::{Action, NavigationId, PeekBuf, RequestId};
