//! How a renderer process asks for bytes it cannot fetch itself.
//!
//! A sandboxed renderer holds no network capability: every stylesheet, font and image it
//! needs is a request to the broker, which fetches through the engine's I/O runtime like any
//! other request. This is that request, as one blocking call per URL. Blocking because the
//! renderer is single-threaded and strictly alternates with the broker on one link; there is
//! nothing else it could be doing while it waits.
//!
//! The same trait sits on the broker side of the link ([`BrokeredLoader`]), where it answers
//! those requests through the I/O runtime. In-process rendering does not use it at all: the
//! resource pipeline and the media store fetch through [`crate::net::submit_to_io`] directly.
//!
//! The renderer's layout reaches it through the interfaces the rest of the engine uses:
//! [`LoaderMediaSource`] for images, and plain closures for stylesheets and web fonts.
//!
//! [`BrokeredLoader`]: crate::net::brokered_loader::BrokeredLoader

use gosub_render_pipeline::common::media::MediaSource;
use gosub_shared::subresource::Scope;
use std::fmt;
use std::sync::Arc;
use url::Url;

/// A resource as delivered to the process that asked for it.
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
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LoadError::UnsupportedUrl(url) => write!(f, "unsupported url: {url}"),
            LoadError::Failed(why) => write!(f, "load failed: {why}"),
            LoadError::TimedOut => write!(f, "load timed out"),
        }
    }
}

impl std::error::Error for LoadError {}

/// Fetches a resource on a renderer's behalf.
pub trait ResourceLoader: Send + Sync + fmt::Debug {
    /// Fetch `url`, blocking until the resource arrives or the attempt fails.
    fn load(&self, url: &Url) -> Result<LoadedResource, LoadError>;

    /// The document the next loads are made for, so a loader that fetches on
    /// a page's behalf can send its `Referer` and apply the document's own
    /// policy (a `file:` page may load `file:` neighbours). Loaders that
    /// serve no page ignore it.
    fn set_document(&self, _url: Option<&Url>) {}

    /// [`load`](Self::load), reduced to what the stylesheet and web-font paths
    /// consume: the bytes and their content type, or nothing.
    fn fetch(&self, url: &str) -> Option<(Option<String>, Vec<u8>)> {
        let url = Url::parse(url).ok()?;
        match self.load(&url) {
            Ok(resource) if resource.is_ok() && !resource.body.is_empty() => {
                Some((resource.content_type, resource.body.to_vec()))
            }
            Ok(resource) => {
                log::warn!("{url} returned status {}", resource.status);
                None
            }
            Err(e) => {
                log::warn!("could not load {url}: {e}");
                None
            }
        }
    }
}

/// A loader that fetches nothing, for contexts with no network at all: tests and
/// any subsystem constructed before its engine has handed one over.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoResourceLoader;

impl ResourceLoader for NoResourceLoader {
    fn load(&self, url: &Url) -> Result<LoadedResource, LoadError> {
        Err(LoadError::UnsupportedUrl(url.to_string()))
    }
}

/// The scope a renderer files its media under in the subresource hand-off.
///
/// The hand-off keys bytes by navigation so that one page's fetch never answers another's.
/// A renderer is its own process with its own hand-off, and [`LoaderMediaSource`] deposits
/// the bytes immediately before the media store takes them, on the one thread the renderer
/// has, so there is never a second page's entry for the same URL to confuse it with.
const RENDERER_SCOPE: Scope = 0;

/// A renderer's [`MediaSource`]: each image is one blocking round trip through `loader`,
/// deposited in the hand-off where the media store is about to look for it.
#[derive(Debug)]
pub struct LoaderMediaSource {
    loader: Arc<dyn ResourceLoader>,
}

impl LoaderMediaSource {
    pub fn new(loader: Arc<dyn ResourceLoader>) -> Self {
        Self { loader }
    }
}

impl MediaSource for LoaderMediaSource {
    fn acquire(&self, url: &str) -> Option<Scope> {
        // Always answered, one way or the other, so the store's `take` never waits out its
        // timeout on an entry nobody is going to fill.
        match self.loader.fetch(url) {
            Some((content_type, bytes)) => {
                gosub_shared::subresource::complete(RENDERER_SCOPE, url, content_type, bytes)
            }
            None => gosub_shared::subresource::abandon(RENDERER_SCOPE, url),
        }
        Some(RENDERER_SCOPE)
    }
}
