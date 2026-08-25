//! How a subsystem asks for bytes it cannot fetch itself.

use std::fmt;
use url::Url;

/// A resource as delivered to the subsystem that asked for it.
#[derive(Debug, Clone)]
pub struct LoadedResource {
    /// HTTP status, or 200 for schemes that have no status of their own.
    pub status: u16,
    /// The `Content-Type` header, when the response carried one. A hint only:
    /// callers are expected to sniff rather than trust it.
    pub content_type: Option<String>,
    pub body: bytes::Bytes,
}

impl LoadedResource {
    pub fn is_ok(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

/// Why a load did not produce bytes.
#[derive(Debug, Clone)]
pub enum LoadError {
    /// The URL was malformed, or its scheme is not one this loader serves.
    UnsupportedUrl(String),
    /// The request reached the network and came back unusable.
    Failed(String),
    /// No reply arrived in time. Distinct from [`Failed`](LoadError::Failed)
    /// because it usually means the broker is starved rather than the resource
    /// being unavailable.
    TimedOut,
    /// The resource is being fetched but is not here yet; ask again later.
    /// A loader that answers this never blocks on the network, and the
    /// caller must not treat it as a failure (in particular, not cache it).
    Pending,
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LoadError::UnsupportedUrl(url) => write!(f, "unsupported url: {url}"),
            LoadError::Failed(why) => write!(f, "load failed: {why}"),
            LoadError::TimedOut => write!(f, "load timed out"),
            LoadError::Pending => write!(f, "load pending"),
        }
    }
}

impl std::error::Error for LoadError {}

/// Fetches a resource on a subsystem's behalf.
pub trait ResourceLoader: Send + Sync + fmt::Debug {
    /// Fetch `url`, blocking until the resource arrives or the attempt fails.
    fn load(&self, url: &Url) -> Result<LoadedResource, LoadError>;
}

/// A loader that fetches nothing, for contexts with no network at all: tests,
/// measurement-only layout passes, and any subsystem constructed before its
/// engine has handed one over.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoResourceLoader;

impl ResourceLoader for NoResourceLoader {
    fn load(&self, url: &Url) -> Result<LoadedResource, LoadError> {
        Err(LoadError::UnsupportedUrl(url.to_string()))
    }
}
