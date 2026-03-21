use bytes::Bytes;
use serde::{Deserialize, Serialize};
use std::fmt::Display;
use std::ops::Deref;
use uuid::Uuid;

/// A small buffer that contains the first bytes of a stream.
/// This is used to "peek" into the stream, to determine the content type.
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

    pub fn to_vec(&self) -> Vec<u8> {
        self.0.to_vec()
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

/// A unique identifier for a network request
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

/// A zone identifier for isolating network requests
#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ZoneId(pub Uuid);

impl ZoneId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for ZoneId {
    fn default() -> Self {
        Self::new()
    }
}

impl Display for ZoneId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<uuid::Uuid> for ZoneId {
    fn from(uuid: uuid::Uuid) -> Self {
        Self(uuid)
    }
}

impl From<String> for ZoneId {
    fn from(s: String) -> Self {
        let uuid = uuid::Uuid::parse_str(&s).unwrap_or_else(|_| uuid::Uuid::new_v4());
        Self(uuid)
    }
}

/// A unique identifier for a browser tab
#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct TabId(pub Uuid);

impl TabId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for TabId {
    fn default() -> Self {
        Self::new()
    }
}

impl Display for TabId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
