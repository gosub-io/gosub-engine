use crate::types::{DecisionToken, PeekBuf};
use http::HeaderMap;
use std::time::Duration;
use url::Url;

/// Events that are emitted by the net::fetch() functions
#[derive(Debug)]
pub enum NetEvent {
    /// Io error happened
    Io { message: String },
    /// Warning happened
    Warning { url: Url, message: String },
    /// Resource is started to load
    Started { url: Url },
    /// Resource is redirected to another URL
    Redirected { from: Url, to: Url, status: u16 },
    /// Response headers are received
    ResponseHeaders { url: Url, status: u16, headers: HeaderMap },
    /// Progress updates, how many bytes already read
    Progress {
        received_bytes: u64,
        expected_length: Option<u64>,
        elapsed: Duration,
    },
    /// Resource is finished
    Finished {
        received_bytes: u64,
        elapsed: Duration,
        url: Url,
    },
    /// Resource failed to fetch
    Failed { url: Url, error: anyhow::Error },
    /// Resource fetching was cancelled
    Cancelled { url: Url, reason: &'static str },
    /// Resource top has been loaded, and UA needs to decide what to do next
    DecisionRequired {
        url: Url,
        status: u16,
        headers: HeaderMap,
        content_length: Option<u64>,
        content_type: Option<String>,
        peek_buf: PeekBuf,
        token: DecisionToken,
    },
}
