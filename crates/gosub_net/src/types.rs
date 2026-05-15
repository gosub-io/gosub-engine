use bytes::Bytes;
use std::fmt::Display;
use std::ops::Deref;
use uuid::Uuid;

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

/// Opaque ID for a document sub-resource load group
pub type DocumentId = u64;
/// Opaque ID for a background prefetch task
pub type PrefetchId = u64;
/// Opaque ID for a miscellaneous background task
pub type TaskId = u64;

/// One logical request chain (stable across redirects)
#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub struct RequestId(pub Uuid);

impl RequestId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for RequestId {
    fn default() -> Self {
        Self::new()
    }
}

impl Display for RequestId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
