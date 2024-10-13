use ureq::Agent;

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
    type Error = ureq::Error;

    fn new() -> Self {
        Agent::new().into()
    }

    async fn get(&self, url: &str) -> gosub_shared::types::Result<Response> {
        let response = self.agent.get(url).call()?;
        response.try_into()
    }

    async fn get_req(&self, _req: &Request) -> gosub_shared::types::Result<Response> {
        todo!()
    }
}

fn get_headers(response: &ureq::Response) -> Headers {
    let names = response.headers_names();

    let mut headers = Headers::with_capacity(names.len());

    for name in names {
        let header = response.header(&name).unwrap_or_default().to_string();

        headers.set(name, header);
    }

    headers
}

impl TryFrom<ureq::Response> for Response {
    type Error = anyhow::Error;

    fn try_from(value: ureq::Response) -> std::result::Result<Self, Self::Error> {
        let body = Vec::with_capacity(
            value
                .header("Content-Length")
                .map(|s| s.parse().unwrap_or(0))
                .unwrap_or(0),
        );

        let mut this = Self {
            status: value.status(),
            status_text: value.status_text().to_string(),
            version: value.http_version().to_string(),
            headers: get_headers(&value),
            body,
            cookies: Default::default(),
        };

        value.into_reader().read_to_end(&mut this.body)?;

        Ok(this)
    }
}
