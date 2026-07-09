use crate::events::{EngineEvent, IoCommand, TabCommand};

pub use gosub_sonar::types::{PeekBuf, RequestId};
use std::fmt::Display;
use uuid::Uuid;

/// What the engine should do with a response once the UA has decided
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// Engine will render the stream
    Render,
    /// Stream will be downloaded to the specified path
    Download { dest: std::path::PathBuf },
    /// Stream will be opened in an external application
    OpenExternal,
    /// Stream will be cancelled
    Cancel,
    /// Stream will be rendered and mirrored to the specified path
    RenderAndMirror { dest: std::path::PathBuf },
    /// Stream will be shown as source (for HTML documents)
    ViewSource,
}

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
pub type IoChannel = tokio::sync::mpsc::UnboundedSender<IoCommand>;
pub type TabChannel = tokio::sync::mpsc::Sender<TabCommand>;
