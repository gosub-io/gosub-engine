use crate::decision_hub::DecisionToken;
use crate::types::PeekBuf;
use http::HeaderMap;
use std::time::Duration;
use url::Url;

/// Events that are send by the net::fetch() functions
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
        // How many bytes received in this resource
        received_bytes: u64,
        // Expected length of the resource (if known)
        expected_length: Option<u64>,
        // Time spent loading so far on this resource
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
        /// The URL of the resource
        url: Url,
        /// The HTTP status code
        status: u16,
        /// The HTTP headers
        headers: HeaderMap,
        /// The content length, if known
        content_length: Option<u64>,
        /// The content type, if known
        content_type: Option<String>,
        /// The first few bytes of the response body
        peek_buf: PeekBuf,
        /// The decision token to correlate the decision with the response
        token: DecisionToken,
    },
}
