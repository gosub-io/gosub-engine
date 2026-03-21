use crate::http::fetcher::RequestAgent;
use crate::http::headers::Headers;
use crate::http::request::Request;
use crate::http::response::Response;
use reqwest::Client;
use std::fmt::{Debug, Formatter};

pub struct ReqwestAgent {
    client: Client,
}

impl Debug for ReqwestAgent {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReqwestAgent").finish()
    }
}

impl RequestAgent for ReqwestAgent {
    type Error = reqwest::Error;

    fn new() -> Self {
        Self {
            client: Client::new(),
        }
    }

    async fn get(&self, url: &str) -> anyhow::Result<Response> {
        let resp = self.client.get(url).send().await?;

        let status = resp.status().as_u16();
        let status_text = resp.status().canonical_reason().unwrap_or("").to_string();
        let version = format!("{:?}", resp.version());
        let headers = build_headers(resp.headers());
        let body = resp.bytes().await?.to_vec();

        Ok(Response {
            status,
            status_text,
            version,
            headers,
            cookies: Default::default(),
            body,
        })
    }

    async fn get_req(&self, req: &Request) -> anyhow::Result<Response> {
        let mut builder = self.client.request(
            req.method.parse().unwrap_or(reqwest::Method::GET),
            &req.uri,
        );

        for (key, value) in req.headers.all() {
            builder = builder.header(key.as_str(), value.as_str());
        }

        if !req.body.is_empty() {
            builder = builder.body(req.body.clone());
        }

        let resp = builder.send().await?;

        let status = resp.status().as_u16();
        let status_text = resp.status().canonical_reason().unwrap_or("").to_string();
        let version = format!("{:?}", resp.version());
        let headers = build_headers(resp.headers());
        let body = resp.bytes().await?.to_vec();

        Ok(Response {
            status,
            status_text,
            version,
            headers,
            cookies: Default::default(),
            body,
        })
    }
}

fn build_headers(header_map: &reqwest::header::HeaderMap) -> Headers {
    let mut headers = Headers::with_capacity(header_map.len());
    for (name, value) in header_map {
        headers.set(name.as_str(), value.to_str().unwrap_or_default());
    }
    headers
}
