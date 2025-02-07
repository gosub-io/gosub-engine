use ureq::{http, Agent, Body};

use crate::http::fetcher::RequestAgent;
use crate::http::headers::Headers;
use crate::http::request::Request;
use crate::http::response::Response;

#[derive(Debug)]
pub struct UreqAgent {
    agent: Agent,
}

impl From<Agent> for UreqAgent {
    fn from(value: Agent) -> Self {
        Self { agent: value }
    }
}

impl RequestAgent for UreqAgent {
    type Error = http::Error;

    fn new() -> Self {
        Agent::new_with_defaults().into()
    }

    async fn get(&self, url: &str) -> gosub_shared::types::Result<Response> {
        let response = self.agent.get(url).call()?;
        response.try_into()
    }

    async fn get_req(&self, _req: &Request) -> gosub_shared::types::Result<Response> {
        todo!()
    }
}

fn get_headers(http_headers: &http::header::HeaderMap) -> Headers {
    let mut headers = Headers::with_capacity(http_headers.len());

    for (name, value) in http_headers.iter() {
        headers.set(name.as_str(), value.to_str().unwrap_or_default());
    }

    headers
}

impl TryFrom<http::response::Response<Body>> for Response {
    type Error = anyhow::Error;

    fn try_from(mut response: http::response::Response<Body>) -> Result<Self, Self::Error> {
        Ok(Self {
            status: response.status().as_u16(),
            status_text: response.status().to_string(),
            version: match response.version() {
                http::Version::HTTP_09 => "http/0.9".into(),
                http::Version::HTTP_10 => "http/1.0".into(),
                http::Version::HTTP_11 => "http/1.1".into(),
                http::Version::HTTP_2 => "http/2.0".into(),
                http::Version::HTTP_3 => "http/3.0".into(),
                _ => "http/1.0".into(),
            },
            headers: get_headers(&response.headers()),
            body: response.body_mut().read_to_vec()?,
            cookies: Default::default(),
        })
    }
}
