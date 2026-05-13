use crate::http::response::Response;
use cow_utils::CowUtils;
use gosub_shared::types::Result;
use std::collections::HashMap;
use std::time::Duration;
use url::Url;

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// Performs a blocking HTTP GET, handling both `file://` and `http(s)://` URLs.
/// Headers in the returned [`Response`] are stored with lowercase keys.
pub fn get(url: &Url) -> Result<Response> {
    if url.scheme() == "file" {
        let path = &url.as_str()[7..];
        let body = std::fs::read(path)?;
        return Ok(Response::from(body));
    }

    let client = reqwest::blocking::Client::builder()
        .timeout(DEFAULT_TIMEOUT)
        .use_rustls_tls()
        .build()?;

    let resp = client.get(url.as_str()).send()?;
    let status = resp.status().as_u16();
    let status_text = resp.status().canonical_reason().unwrap_or("").to_string();

    let headers: HashMap<String, String> = resp
        .headers()
        .iter()
        .filter_map(|(k, v)| {
            v.to_str()
                .ok()
                .map(|v| (k.as_str().cow_to_lowercase().into_owned(), v.to_string()))
        })
        .collect();

    let body = resp.bytes()?.to_vec();

    Ok(Response {
        status,
        status_text,
        version: "HTTP/1.1".to_string(),
        headers,
        cookies: HashMap::new(),
        body,
    })
}
