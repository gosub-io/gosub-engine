use crate::http::response::Response;
use cookie::Cookie;
use cow_utils::CowUtils;
use gosub_shared::types::Result;
use std::collections::HashMap;
use std::io::Read;
use std::sync::OnceLock;
use std::time::Duration;
use url::Url;

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_BODY_SIZE: u64 = 10 * 1024 * 1024; // 10 MB

static HTTP_CLIENT: OnceLock<reqwest::blocking::Client> = OnceLock::new();

fn get_client() -> &'static reqwest::blocking::Client {
    HTTP_CLIENT.get_or_init(|| {
        reqwest::blocking::Client::builder()
            .timeout(DEFAULT_TIMEOUT)
            .use_rustls_tls()
            .build()
            .expect("Failed to build HTTP client")
    })
}

/// Performs a blocking HTTP GET, handling both `file://` and `http(s)://` URLs.
/// Headers in the returned [`Response`] are stored with lowercase keys.
pub fn get(url: &Url) -> Result<Response> {
    if url.scheme() == "file" {
        // to_file_path() handles well-formed file:///absolute/path URLs.
        // For relative-style file://relative/path URLs the host holds the first
        // path segment, so we fall back to concatenating host + path.
        let path = url.to_file_path().unwrap_or_else(|_| {
            let host = url.host_str().unwrap_or("");
            std::path::PathBuf::from(format!("{}{}", host, url.path()))
        });
        let mut body = Vec::new();
        std::fs::File::open(&path)?
            .take(MAX_BODY_SIZE + 1)
            .read_to_end(&mut body)?;
        if body.len() as u64 > MAX_BODY_SIZE {
            anyhow::bail!("File size exceeds maximum size of {} bytes", MAX_BODY_SIZE);
        }
        return Ok(Response::from(body));
    }

    let resp = get_client().get(url.as_str()).send()?;
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

    // Reject early if Content-Length declares an oversized body.
    if let Some(cl) = resp.headers().get("content-length") {
        if let Ok(size) = cl.to_str().unwrap_or("").parse::<u64>() {
            if size > MAX_BODY_SIZE {
                anyhow::bail!("Response body exceeds maximum size of {} bytes", MAX_BODY_SIZE);
            }
        }
    }

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
            Cookie::parse(s.to_owned())
                .ok()
                .map(|c| (c.name().to_owned(), c.value().to_owned()))
        })
        .collect();

    let mut body = Vec::new();
    resp.take(MAX_BODY_SIZE + 1).read_to_end(&mut body)?;
    if body.len() as u64 > MAX_BODY_SIZE {
        anyhow::bail!("Response body exceeds maximum size of {} bytes", MAX_BODY_SIZE);
    }

    Ok(Response {
        status,
        status_text,
        version,
        headers,
        cookies,
        body,
    })
}
