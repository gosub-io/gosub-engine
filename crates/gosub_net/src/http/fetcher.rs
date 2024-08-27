use super::response::Response;
use crate::http::request::Request;
use anyhow::bail;
use gosub_shared::types::Result;
use url::{ParseError, Url};

pub struct Fetcher {
    base_url: Url,
    client: ureq::Agent,
}

impl Fetcher {
    pub fn new(base: Url) -> Self {
        Self {
            base_url: base,
            client: ureq::Agent::new(),
        }
    }

    pub fn get_url(&self, url: &Url) -> Result<Response> {
        let scheme = url.scheme();

        let resp = if scheme == "http" || scheme == "https" {
            let response = self.client.get(url.as_str()).call()?;

            response.try_into()?
        } else if scheme == "file" {
            let path = &url.as_str()[7..];

            let body = std::fs::read(path)?;

            Response::from(body)
        } else {
            bail!("Unsupported scheme")
        };

        Ok(resp)
    }

    pub fn get(&self, url: &str) -> Result<Response> {
        let url = self.parse_url(url)?;

        self.get_url(&url)
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
