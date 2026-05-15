//! Demonstrates `simple_get` — a one-shot GET that returns the body as bytes.
//!
//! Run with:
//!   cargo run -p gosub_net --example simple_fetch -- https://example.org
//!   cargo run -p gosub_net --example simple_fetch -- file:///etc/hostname

use gosub_net::net::simple::simple_get;
use url::Url;

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let raw = std::env::args().nth(1).unwrap_or_else(|| "https://example.org".to_string());
    let url = Url::parse(&raw)?;

    println!("Fetching {url} ...");

    let bytes = simple_get(&url).await?;

    println!("Received {} bytes", bytes.len());
    if let Ok(text) = std::str::from_utf8(&bytes[..bytes.len().min(512)]) {
        println!("{text}");
    }

    Ok(())
}
