use anyhow::bail;
use gosub_shared::types::Result;
use url::{ParseError, Url};

use super::response::Response;

#[derive(Debug)]
pub struct Fetcher {
    base_url: Url,
    client: reqwest::Client,
}

impl Fetcher {
    #[must_use]
    pub fn new(base: Url) -> Self {
        #[cfg(not(target_arch = "wasm32"))]
        let client = reqwest::Client::builder()
            .use_rustls_tls()
            .build()
            .expect("failed to build HTTP client");

        #[cfg(target_arch = "wasm32")]
        let client = reqwest::Client::new();

        Self { base_url: base, client }
    }

    #[must_use]
    pub fn base(&self) -> &Url {
        &self.base_url
    }

    pub async fn get_url(&self, url: &Url) -> Result<Response> {
        let scheme = url.scheme();

        #[cfg(not(target_arch = "wasm32"))]
        if scheme == "file" {
            let path = &url.as_str()[7..];
            let body = std::fs::read(path)?;
            return Ok(Response::from(body));
        }

        if scheme != "http" && scheme != "https" {
            bail!("Unsupported scheme: {}", scheme);
        }

        let resp = self.client.get(url.as_str()).send().await?;
        let status = resp.status().as_u16();
        let body = resp.bytes().await?.to_vec();
        Ok(Response {
            status,
            body,
            ..Response::default()
        })
    }

    pub async fn get(&self, url: &str) -> Result<Response> {
        self.get_url(&self.parse_url(url)?).await
    }

    pub fn parse_url(&self, url: &str) -> Result<Url> {
        let mut parsed_url = Url::parse(url);
        if parsed_url == Err(ParseError::RelativeUrlWithoutBase) {
            parsed_url = self.base_url.join(url);
        }
        Ok(parsed_url?)
    }
}
