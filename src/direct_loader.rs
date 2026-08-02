//! A [`ResourceLoader`] that fetches directly, for tools that are not the engine.
//!
//! The engine deliberately gives its subsystems no ambient network access: a
//! parser or layout pass loads what it is handed a loader for, and nothing else
//! (see `gosub_interface::resource_loader`). Inside the engine that loader routes
//! through the I/O runtime, where identity and cookies apply and where the
//! network will eventually live in its own process.
//!
//! The standalone CLI tools here have no engine, no tab and no cookie jar — they
//! parse one document and exit. They still need external stylesheets to resolve,
//! so they supply this: a blocking fetch straight to the network, named plainly
//! so its use is a visible choice rather than a default that quietly reintroduces
//! ambient networking into the engine.

use gosub_interface::resource_loader::{LoadError, LoadedResource, ResourceLoader};
use url::Url;

/// Fetches over the network with no brokering, identity or cookies.
#[derive(Debug, Clone, Copy, Default)]
pub struct DirectResourceLoader;

impl ResourceLoader for DirectResourceLoader {
    fn load(&self, url: &Url) -> Result<LoadedResource, LoadError> {
        let response = gosub_sonar::net::simple::sync_fetch(url).map_err(|e| LoadError::Failed(e.to_string()))?;

        Ok(LoadedResource {
            status: response.status,
            content_type: response.headers.get("content-type").cloned(),
            body: bytes::Bytes::from(response.body),
        })
    }
}
