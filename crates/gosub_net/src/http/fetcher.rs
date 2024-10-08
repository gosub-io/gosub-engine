use anyhow::bail;
use std::error::Error;
use std::fmt::Debug;
use std::future::Future;
use url::{ParseError, Url};

use gosub_shared::types::Result;

use crate::http::request::Request;
use crate::http::request_impl::RequestImpl;

use super::response::Response;

pub trait RequestAgent: Debug {
    type Error: Error;

    fn new() -> Self;

    fn get(&self, url: &str) -> impl Future<Output = Result<Response>>;

    fn get_req(&self, req: &Request) -> impl Future<Output = Result<Response>>;
}

#[derive(Debug)]
pub struct Fetcher {
    base_url: Url,
    client: RequestImpl,
}

impl Fetcher {
    pub fn new(base: Url) -> Self {
        Self {
            base_url: base,
            client: RequestImpl::new(),
        }
    }

    pub async fn get_url(&self, url: &Url) -> Result<Response> {
        let scheme = url.scheme();

        let resp = if scheme == "http" || scheme == "https" {
            self.client.get(url.as_str()).await?
        } else if scheme == "file" {
            let path = &url.as_str()[7..];

            let body = std::fs::read(path)?;

            Response::from(body)
        } else {
            bail!("Unsupported scheme")
        };

        Ok(resp)
    }

    pub async fn get(&self, url: &str) -> Result<Response> {
        let url = self.parse_url(url)?;

        self.get_url(&url).await
    }

    pub fn get_req(&self, _url: &Request) {
        todo!()
    }

    fn parse_url(&self, url: &str) -> Result<Url> {
        let mut parsed_url = Url::parse(url);

        if parsed_url == Err(ParseError::RelativeUrlWithoutBase) {
            parsed_url = self.base_url.join(url);
        }

        Ok(parsed_url?)
    }
}
