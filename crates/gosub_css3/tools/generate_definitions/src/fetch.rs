//! The one way this tool talks to the network.
//!
//! Everything HTTP in the workspace goes through gosub-sonar, this generator included. Sonar
//! sets what a Gosub request looks like on the wire, the user agent included - GitHub's API
//! refuses a request without one - and a tool with its own client would not pick up changes
//! made there.

use anyhow::{Context, Result};

/// GET `url`, returning the body. Non-2xx responses are errors.
pub fn get(url: &str) -> Result<Vec<u8>> {
    let parsed = url::Url::parse(url).with_context(|| format!("invalid URL {url}"))?;
    let body = gosub_sonar::sync_get(&parsed).with_context(|| format!("fetching {url}"))?;
    Ok(body.to_vec())
}
