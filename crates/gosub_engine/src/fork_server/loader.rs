//! The renderer's [`ResourceLoader`]: every load is a blocking round trip to
//! the broker.

use crate::fork_server::protocol::{FromRenderer, ResourceReply};
use gosub_interface::resource_loader::{LoadError, LoadedResource, ResourceLoader};
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
}

impl ResourceLoader for ForkedResourceLoader {
    fn load(&self, url: &Url) -> Result<LoadedResource, LoadError> {
        let slot = self.link.lock();
        let Some(link) = slot.as_ref() else {
            return Err(LoadError::Failed(
                "renderer loader is not connected (loads only exist inside a forked renderer)".into(),
            ));
        };
        let mut link = link.lock();

        link.send(&FromRenderer::NeedResource { url: url.to_string() })
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
            ResourceReply::Failed(reason) => Err(LoadError::Failed(reason)),
        }
    }
}
