use anyhow::Result;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use url::Url;

/// A minimal HTTP response used by the Fetcher trait
pub struct FetchResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

impl FetchResponse {
    pub fn is_ok(&self) -> bool {
        self.status >= 200 && self.status < 300
    }
}

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Trait abstracting the HTTP fetcher so `gosub_interface` does not depend on `gosub_net`.
pub trait Fetcher: Send + Sync {
    /// Fetch the given URL and return the response
    fn get_url<'a>(&'a self, url: &'a Url) -> BoxFuture<'a, Result<FetchResponse>>;

    /// Fetch the given URL string and return the response
    fn get<'a>(&'a self, url: &'a str) -> BoxFuture<'a, Result<FetchResponse>>;

    /// Parse a (possibly relative) URL against the base URL of this fetcher
    fn parse_url(&self, url: &str) -> Result<Url>;

    /// Returns the base URL
    fn base(&self) -> &Url;
}

/// Helper type alias
pub type SharedFetcher = Arc<dyn Fetcher>;
