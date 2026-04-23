use crate::events::{EngineEvent, IoCommand, TabCommand};
use bytes::Bytes;
use std::fmt::Display;
use std::ops::Deref;
use uuid::Uuid;

// Defined channels for communication
pub type EventChannel = tokio::sync::broadcast::Sender<EngineEvent>;
pub type IoChannel = tokio::sync::mpsc::UnboundedSender<IoCommand>;
pub type TabChannel = tokio::sync::mpsc::Sender<TabCommand>;

/// A small buffer that contains the first bytes of a stream.
/// This is used to "peek" into the stream, to determine the content type
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PeekBuf(Bytes);

impl PeekBuf {
    pub fn from_vec(vec: Vec<u8>) -> Self {
        Self(Bytes::from(vec))
    }

    pub fn from_slice(s: &[u8]) -> Self {
        Self(Bytes::copy_from_slice(s))
    }

    pub fn empty() -> Self {
        Self(Bytes::new())
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.0
    }

    pub fn as_bytes(&self) -> &Bytes {
        &self.0
    }

    pub fn into_bytes(self) -> Bytes {
        self.0
    }
}

impl AsRef<[u8]> for PeekBuf {
    fn as_ref(&self) -> &[u8] {
        self.as_slice()
    }
}
impl Deref for PeekBuf {
    type Target = [u8];
    fn deref(&self) -> &Self::Target {
        self.as_slice()
    }
}

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

// one logical request chain (stable across redirects)
#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub struct RequestId(pub Uuid);

impl RequestId {
    #[allow(unused)]
    pub(crate) fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Display for RequestId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
