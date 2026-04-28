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
        Self {
            base_url: base,
            client: reqwest::Client::builder()
                .use_rustls_tls()
                .build()
                .expect("failed to build HTTP client"),
        }
    }

    #[must_use]
    pub fn base(&self) -> &Url {
        &self.base_url
    }

    pub async fn get_url(&self, url: &Url) -> Result<Response> {
        let scheme = url.scheme();
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

    pub async fn get_url(&self, url: &Url) -> Result<Response> {
        use anyhow::anyhow;
        use js_sys::{ArrayBuffer, Uint8Array};
        use wasm_bindgen_futures::JsFuture;
        use web_sys::wasm_bindgen::JsCast;
        use web_sys::{RequestInit, RequestMode};

        let scheme = url.scheme();
        if scheme == "file" {
            bail!("File scheme not supported on wasm");
        }
        if scheme != "http" && scheme != "https" {
            bail!("Unsupported scheme: {}", scheme);
        }

        let opts = RequestInit::new();
        opts.set_method("GET");
        opts.set_mode(RequestMode::Cors);

        let request = web_sys::Request::new_with_str_and_init(url.as_str(), &opts).map_err(|e| anyhow!("{e:?}"))?;

        let window = web_sys::window().ok_or_else(|| anyhow!("No window"))?;
        let resp_val = JsFuture::from(window.fetch_with_request(&request))
            .await
            .map_err(|e| anyhow!("{e:?}"))?;

        let resp: web_sys::Response = resp_val.dyn_into().map_err(|e| anyhow!("{e:?}"))?;
        let status = resp.status();

        let buf = JsFuture::from(resp.array_buffer().map_err(|e| anyhow!("{e:?}"))?)
            .await
            .map_err(|e| anyhow!("{e:?}"))?;

        let array: ArrayBuffer = buf.dyn_into().map_err(|e| anyhow!("{e:?}"))?;
        let body = Uint8Array::new(&array).to_vec();

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
