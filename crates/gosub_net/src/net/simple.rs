use bytes::Bytes;
use gosub_shared::types::Result;
use url::Url;

/// Perform a simple one-shot GET request and return the body as bytes.
/// Handles http, https, and file:// URLs.
/// Use this for standalone callers (renderer, tools) that don't need the full
/// priority-scheduler Fetcher.
pub async fn simple_get(url: &Url) -> Result<Bytes> {
    match url.scheme() {
        "file" => {
            let path = url.as_str().trim_start_matches("file://");
            Ok(Bytes::from(std::fs::read(path)?))
        }
        "http" | "https" => {
            let client = reqwest::Client::builder().use_rustls_tls().build()?;
            let resp = client.get(url.as_str()).send().await?;
            let status = resp.status();
            if !status.is_success() {
                anyhow::bail!("HTTP {status} fetching {url}");
            }
            Ok(resp.bytes().await?)
        }
        scheme => anyhow::bail!("Unsupported URL scheme: {scheme}"),
    }
}
