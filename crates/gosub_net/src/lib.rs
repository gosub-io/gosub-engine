//! Networking functionality
//!
//! This module contains all the networking functionality for the browser. This is normally the
//! low-level implementation of the browser. The networking module is responsible for making HTTP
//! requests, parsing the response and returning the result to the caller.
//!
//! It also contains additional networking components like the DNS resolver.

extern crate gosub_config;

#[cfg(not(target_arch = "wasm32"))]
pub mod dns;
pub mod errors;

// New net modules
pub mod types;
pub mod policy;
pub mod events;
pub mod emitter;
pub mod net_types;
pub mod shared_body;
pub mod utils;
pub mod fs_utils;
pub mod fetch;
pub mod pump;
pub mod decision;
pub mod decision_hub;
pub mod req_ref_tracker;
pub mod fetcher;
pub mod io_types;
pub mod io_runtime;
pub mod spawn;

/// Creates a new HTTP fetcher for the given base URL using the new fetch infrastructure.
/// Returns a `SharedFetcher` for backward compatibility.
#[cfg(not(target_arch = "wasm32"))]
pub fn new_fetcher(base: url::Url) -> gosub_interface::fetcher::SharedFetcher {
    std::sync::Arc::new(SimpleHttpFetcher::new(base))
}

/// A simple HTTP fetcher that implements the `gosub_interface::fetcher::Fetcher` trait
/// using the new `fetch_response_complete` function.
#[cfg(not(target_arch = "wasm32"))]
pub struct SimpleHttpFetcher {
    base_url: url::Url,
    client: std::sync::Arc<reqwest::Client>,
}

#[cfg(not(target_arch = "wasm32"))]
impl SimpleHttpFetcher {
    pub fn new(base: url::Url) -> Self {
        let client = reqwest::Client::builder()
            .use_rustls_tls()
            .build()
            .expect("reqwest client build failed");
        Self {
            base_url: base,
            client: std::sync::Arc::new(client),
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl gosub_interface::fetcher::Fetcher for SimpleHttpFetcher {
    fn get_url<'a>(
        &'a self,
        url: &'a url::Url,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<gosub_interface::fetcher::FetchResponse>> + Send + 'a>> {
        let client = self.client.clone();
        let url = url.clone();
        Box::pin(async move {
            let cancel = tokio_util::sync::CancellationToken::new();
            let null_observer: std::sync::Arc<dyn emitter::NetObserver + Send + Sync> =
                std::sync::Arc::new(emitter::null_emitter::NullEmitter);

            let (meta, body) = fetch::fetch_response_complete(
                client,
                url,
                cancel,
                null_observer,
                None,
                std::time::Duration::from_secs(15),
                Some(std::time::Duration::from_secs(180)),
            )
            .await
            .map_err(|e| anyhow::anyhow!("{}", e))?;

            Ok(gosub_interface::fetcher::FetchResponse {
                status: meta.status,
                body,
            })
        })
    }

    fn get<'a>(
        &'a self,
        url: &'a str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<gosub_interface::fetcher::FetchResponse>> + Send + 'a>> {
        let parsed = self.parse_url(url);
        Box::pin(async move {
            let url = parsed?;
            let client = reqwest::Client::builder()
                .use_rustls_tls()
                .build()
                .expect("reqwest client build failed");
            let cancel = tokio_util::sync::CancellationToken::new();
            let null_observer: std::sync::Arc<dyn emitter::NetObserver + Send + Sync> =
                std::sync::Arc::new(emitter::null_emitter::NullEmitter);

            let (meta, body) = fetch::fetch_response_complete(
                std::sync::Arc::new(client),
                url,
                cancel,
                null_observer,
                None,
                std::time::Duration::from_secs(15),
                Some(std::time::Duration::from_secs(180)),
            )
            .await
            .map_err(|e| anyhow::anyhow!("{}", e))?;

            Ok(gosub_interface::fetcher::FetchResponse {
                status: meta.status,
                body,
            })
        })
    }

    fn parse_url(&self, url: &str) -> anyhow::Result<url::Url> {
        let mut parsed = url::Url::parse(url);
        if parsed == Err(url::ParseError::RelativeUrlWithoutBase) {
            parsed = self.base_url.join(url);
        }
        Ok(parsed?)
    }

    fn base(&self) -> &url::Url {
        &self.base_url
    }
}
