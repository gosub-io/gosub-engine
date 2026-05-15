use crate::events::{EngineEvent, IoCommand, TabCommand};

pub use gosub_net::types::{Action, NavigationId, PeekBuf, RequestId};

// Defined channels for communication
pub type EventChannel = tokio::sync::broadcast::Sender<EngineEvent>;
pub type IoChannel = tokio::sync::mpsc::UnboundedSender<IoCommand>;
pub type TabChannel = tokio::sync::mpsc::Sender<TabCommand>;

