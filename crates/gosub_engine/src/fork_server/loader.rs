//! The renderer's [`ResourceLoader`]: every load is a blocking round trip to
//! the broker.

use crate::fork_server::protocol::{FromRenderer, ResourceReply};
use crate::net::resource_loader::{LoadError, LoadedResource, ResourceLoader};
use gosub_ipc::Endpoint;
use parking_lot::Mutex;
use std::sync::Arc;
use url::Url;

/// Shared as `Arc<dyn ResourceLoader>` inside the `MediaStore` and as
/// `Arc<ForkedResourceLoader>` by the fork server, so a forked child can
/// reach `connect` on the very object the store already holds.
pub struct ForkedResourceLoader {
    link: Mutex<Option<Arc<Mutex<Endpoint>>>>,
}

impl std::fmt::Debug for ForkedResourceLoader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ForkedResourceLoader")
            .field("connected", &self.link.lock().is_some())
            .finish()
    }
}

impl ForkedResourceLoader {
    /// A loader with no link yet - safe to embed in shared state pre-fork.
    pub fn disconnected() -> Arc<Self> {
        Arc::new(Self { link: Mutex::new(None) })
    }

    /// Point this (copy-on-write) copy of the loader at the renderer's own
    /// endpoint. Called once, by a forked child, before it renders; the same
    /// endpoint later carries its `Rendered` result, which is safe because
    /// the renderer is single-threaded and strictly alternates.
    pub fn connect(&self, link: Arc<Mutex<Endpoint>>) {
        *self.link.lock() = Some(link);
    }

    /// The same link, asking for resources the render can do without for
    /// now (images): the broker answers immediately, with the bytes or
    /// [`LoadError::Pending`], and fetches in the background.
    pub fn deferred(self: &Arc<Self>) -> Arc<DeferredForkedLoader> {
        Arc::new(DeferredForkedLoader(Arc::clone(self)))
    }

    fn load_with(&self, url: &Url, deferred: bool) -> Result<LoadedResource, LoadError> {
        let slot = self.link.lock();
        let Some(link) = slot.as_ref() else {
            return Err(LoadError::Failed(
                "renderer loader is not connected (loads only exist inside a forked renderer)".into(),
            ));
        };
        let mut link = link.lock();

        link.send(&FromRenderer::NeedResource {
            url: url.to_string(),
            deferred,
        })
        .map_err(|e| LoadError::Failed(format!("could not reach the broker: {e}")))?;
        // No timeout on this recv: the renderer filter has no `setsockopt`.
        // A dead parent is an EOF; a wedged one is bounded by the broker's
        // clocks, which tear this whole process family down.
        match link
            .recv::<ResourceReply>()
            .map_err(|e| LoadError::Failed(format!("the broker never answered: {e}")))?
        {
            ResourceReply::Ok {
                status,
                content_type,
                body,
            } => Ok(LoadedResource {
                status,
                content_type,
                body: bytes::Bytes::from(body),
            }),
            ResourceReply::Shared {
                status,
                content_type,
                len,
            } => Ok(LoadedResource {
                status,
                content_type,
                body: bytes::Bytes::from(receive_shared_body(&mut link, len)?),
            }),
            ResourceReply::Failed(reason) => Err(LoadError::Failed(reason)),
            ResourceReply::Pending => Err(LoadError::Pending),
        }
    }
}

/// The body of a [`ResourceReply::Shared`]: the sealed memfd that follows it on
/// `link`, read out whole.
pub(crate) fn receive_shared_body(link: &mut Endpoint, len: u64) -> Result<Vec<u8>, LoadError> {
    let fd = link
        .rx
        .recv_fd()
        .map_err(|e| LoadError::Failed(format!("the broker's shared body never arrived: {e}")))?;
    let len = usize::try_from(len).map_err(|_| LoadError::Failed(format!("a {len}-byte body is too large")))?;
    gosub_ipc::shm::read_sealed_blob(fd, len).map_err(|e| LoadError::Failed(format!("reading the shared body: {e}")))
}

/// [`ForkedResourceLoader`] in deferred mode; see [`ForkedResourceLoader::deferred`].
pub struct DeferredForkedLoader(Arc<ForkedResourceLoader>);

impl std::fmt::Debug for DeferredForkedLoader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("DeferredForkedLoader").field(&self.0).finish()
    }
}

impl ResourceLoader for DeferredForkedLoader {
    fn load(&self, url: &Url) -> Result<LoadedResource, LoadError> {
        self.0.load_with(url, true)
    }
}

impl ResourceLoader for ForkedResourceLoader {
    fn load(&self, url: &Url) -> Result<LoadedResource, LoadError> {
        self.load_with(url, false)
    }
}
