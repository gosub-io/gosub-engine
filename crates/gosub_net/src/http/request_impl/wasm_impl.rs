use crate::http::fetcher::RequestAgent;
use crate::http::headers::Headers;
use crate::http::request::Request;
use crate::http::response::Response;
use anyhow::anyhow;
use gosub_shared::types::Result;
use js_sys::{ArrayBuffer, Uint8Array};
use log::info;
use std::error::Error;
use std::fmt::{Debug, Display, Formatter};
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use wasm_bindgen_futures::JsFuture;
use web_sys::wasm_bindgen::JsCast;
use web_sys::{RequestInit, RequestMode};

#[derive(Debug)]
pub struct WasmAgent;

#[derive(Debug)]
pub struct WasmError {
    message: String,
}

impl Display for WasmError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl Error for WasmError {}

impl RequestAgent for WasmAgent {
    type Error = WasmError;

    fn new() -> Self {
        Self
    }

    async fn get(&self, url: &str) -> Result<Response> {
        info!("Fetching: {:?}", url);

        let opts = RequestInit::new();

        opts.set_method("GET");
        opts.set_mode(RequestMode::Cors);

        let req = web_sys::Request::new_with_str_and_init(&url, &opts).map_err(|e| anyhow!("{e:?}"))?;

        let res = fetch(req).await?;

        Ok(res)
    }

    async fn get_req(&self, req: &Request) -> Result<Response> {
        let opts = RequestInit::new();

        opts.set_method(&req.method);
        opts.set_mode(RequestMode::Cors);

        opts.set_body(&req.body.clone().into());

        //TODO: headers, version, cookies

        let req = web_sys::Request::new_with_str_and_init(&req.uri, &opts).map_err(|e| anyhow!("{e:?}"))?;

        fetch(req).await
    }
}

struct UnsafeFuture<F: Future> {
    inner: F,
}

impl<F: Future> From<F> for UnsafeFuture<F> {
    fn from(inner: F) -> Self {
        Self { inner }
    }
}

/// Generally this is NOT safe to do, but in this context, it is
unsafe impl<F: Future> Send for UnsafeFuture<F> {}

impl<F: Future> Future for UnsafeFuture<F> {
    type Output = F::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        //it's going to be fine, I js_sys::Promise
        let pin = unsafe { Pin::new_unchecked(&mut self.get_unchecked_mut().inner) };

        F::poll(pin, cx)
    }
}

async fn fetch(req: web_sys::Request) -> Result<Response> {
    info!("Fetching (worker): {:?}", req.url());

    let window = web_sys::window().ok_or(anyhow!("No window"))?;

    let resp = JsFuture::from(window.fetch_with_request(&req))
        .await
        .map_err(|e| anyhow!("{e:?}"))?;

    let resp: web_sys::Response = resp.dyn_into().map_err(|e| anyhow!("{e:?}"))?;

    // let req_headers = resp.headers();

    let headers = Headers::new();

    // for iter in req_headers.values() {
    //
    //     let iter_value = iter?;
    //
    //
    //
    //
    //     headers.append(&name, &value);
    // }

    let cookies = Default::default();

    let buf = JsFuture::from(resp.array_buffer().map_err(|e| anyhow!("{e:?}"))?)
        .await
        .map_err(|e| anyhow!("{e:?}"))?;

    let array: ArrayBuffer = buf.dyn_into().map_err(|e| anyhow!("{e:?}"))?;

    let body = Uint8Array::new(&array).to_vec();

    info!("Response: {:?}", body.len());
    info!("Status: {:?}", resp.status());
    info!("Status Text: {:?}", resp.status_text());

    Ok(Response {
        status: resp.status(),
        status_text: resp.status_text(),
        version: Default::default(),
        headers,
        cookies,
        body,
    })
}
