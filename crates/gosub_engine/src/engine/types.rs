use crate::events::{EngineEvent, IoCommand, TabCommand};
use std::fmt::Display;
use uuid::Uuid;

pub use gosub_net::types::{PeekBuf, RequestId};

// Defined channels for communication
pub type EventChannel = tokio::sync::broadcast::Sender<EngineEvent>;
pub type IoChannel = tokio::sync::mpsc::UnboundedSender<IoCommand>;
pub type TabChannel = tokio::sync::mpsc::Sender<TabCommand>;

/// Used to send back which action needs to be taken for a navigation request.
/// After the engine reads the headers and the first xKb bytes, it will return
/// a meta object with all information for the user agent to decide what to do.
///
/// It chould be that it's a HTML document, and the UA can decide to that the
/// engine must render it. Or, it can fetch the HTML document, and show it (as raw)
/// on the UI. Or, it could be a binary file, and the UA can decide to download it
/// instead of rendering it.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// Engine will handle the stream
    Render,
    /// Stream will be directly downloaded to the specified path
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
    pub(crate) fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Display for NavigationId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
