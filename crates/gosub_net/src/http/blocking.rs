use crate::http::response::Response;
use cow_utils::CowUtils;
use gosub_shared::types::Result;
use std::collections::HashMap;
use std::time::Duration;
use url::Url;

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_BODY_SIZE: u64 = 10 * 1024 * 1024; // 10 MB

/// Performs a blocking HTTP GET, handling both `file://` and `http(s)://` URLs.
/// Headers in the returned [`Response`] are stored with lowercase keys.
pub fn get(url: &Url) -> Result<Response> {
    if url.scheme() == "file" {
        let path = url
            .to_file_path()
            .map_err(|_| anyhow::anyhow!("Invalid file URL: {}", url))?;
        let body = std::fs::read(&path)?;
        return Ok(Response::from(body));
    }

    let client = reqwest::blocking::Client::builder()
        .timeout(DEFAULT_TIMEOUT)
        .use_rustls_tls()
        .build()?;

    let resp = client.get(url.as_str()).send()?;
    let status = resp.status().as_u16();
    let status_text = resp.status().canonical_reason().unwrap_or("").to_string();
    let version = match resp.version() {
        reqwest::Version::HTTP_10 => "HTTP/1.0",
        reqwest::Version::HTTP_11 => "HTTP/1.1",
        reqwest::Version::HTTP_2 => "HTTP/2",
        reqwest::Version::HTTP_3 => "HTTP/3",
        _ => "HTTP/1.1",
    }
    .to_string();

    let headers: HashMap<String, String> = resp
        .headers()
        .iter()
        .filter_map(|(k, v)| {
            v.to_str()
                .ok()
                .map(|v| (k.as_str().cow_to_lowercase().into_owned(), v.to_string()))
        })
        .collect();

    let cookies: HashMap<String, String> = resp
        .headers()
        .get_all("set-cookie")
        .iter()
        .filter_map(|v| {
            let s = v.to_str().ok()?;
            let kv = s.split(';').next()?;
            let (name, value) = kv.split_once('=')?;
            Some((name.trim().to_string(), value.trim().to_string()))
        })
        .collect();

    let bytes = resp.bytes()?;
    if bytes.len() as u64 > MAX_BODY_SIZE {
        anyhow::bail!("Response body exceeds maximum size of {} bytes", MAX_BODY_SIZE);
    }
    let body = bytes.to_vec();

    Ok(Response {
        status,
        status_text,
        version,
        headers,
        cookies,
        body,
    })
}
