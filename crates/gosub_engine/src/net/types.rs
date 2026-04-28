use crate::engine::types::{PeekBuf, RequestId};
use crate::html::DummyDocument;
use crate::net::req_ref_tracker::RequestReference;
use crate::net::shared_body::SharedBody;
use crate::net::utils::{normalize_url, short_hash, BytesAsyncReader};
use bytes::Bytes;
use http::{header, HeaderMap, Method};
use std::fmt::{Debug, Display};
use std::hash::Hash;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use tokio::io::{AsyncRead, ReadBuf};
use tokio_util::sync::CancellationToken;
use url::Url;

/// A BodyStream is an async reader that can be used to read the body of a response.
pub struct BodyStream {
    /// Inner reader
    inner: Pin<Box<dyn AsyncRead + Send + 'static>>,
    /// Content length (if known)
    pub len: Option<u64>,
    /// True when the stream is seekable (most often not, unless it's backed by a memory buffer)
    pub is_seekable: bool,
    /// Can be cloned to create a new independent stream starting at the beginning
    pub clonable: bool,
}

impl Debug for BodyStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BodyStream")
            .field("len", &self.len)
            .field("is_seekable", &self.is_seekable)
            .field("clonable", &self.clonable)
            .finish()
    }
}

impl BodyStream {
    pub fn new(inner: Pin<Box<dyn AsyncRead + Send + 'static>>, len: Option<u64>) -> Self {
        Self {
            inner,
            len,
            is_seekable: false,
            clonable: false,
        }
    }

    /// Converts a series of bytes into a body stream
    pub fn from_bytes(bytes: Bytes) -> Self {
        let len = bytes.len() as u64;
        let reader = Box::pin(BytesAsyncReader { data: bytes, pos: 0 });

        Self {
            inner: reader,
            len: Some(len),
            is_seekable: true, // It's a buffer so we can seek it
            clonable: true,    // It's a buffer so we can clone it
        }
    }
}

impl AsyncRead for BodyStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        self.inner.as_mut().poll_read(cx, buf)
    }
}

/// Metadata returned by the FetchResult
#[derive(Clone, Debug)]
pub struct FetchResultMeta {
    /// Final URL after redirects
    pub final_url: Url,
    /// HTTP status code
    pub status: u16,
    /// HTTP status reason phrase
    pub status_text: String,
    /// Response headers
    pub headers: HeaderMap,
    /// Length of the content (if known from headers)
    pub content_length: Option<u64>,
    /// Content-Type header (if any)
    pub content_type: Option<String>,
    /// True if the response has a body (e.g. HEAD requests do not)
    pub has_body: bool,
}

impl Default for FetchResultMeta {
    fn default() -> Self {
        Self {
            final_url: Url::parse("about:blank").unwrap(),
            status: 0,
            status_text: "".into(),
            headers: HeaderMap::new(),
            content_length: None,
            content_type: None,
            has_body: true,
        }
    }
}

/// Priority of the scheduled request. Documents usually have high priority, while images have low.
/// Currently, the scheduler uses a round-robin system to load resources
#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)]
pub enum Priority {
    High,
    Normal,
    Low,
    Idle,
}

impl Display for Priority {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Priority::High => "High",
            Priority::Normal => "Normal",
            Priority::Low => "Low",
            Priority::Idle => "Idle",
        };

        f.write_str(s)
    }
}

/// Defines the different resource types that are available for loading
#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub enum ResourceKind {
    Document,
    Stylesheet,
    Script { blocking: bool },
    Image,
    Font,
    Media,
    Xhr,
    Fetch,
    WebSocket,
    Other,
}

/// A fetch key data is a key that is used to find out if two requests want to fetch the same resource.
/// If this is true, the requests are bundled so only once the resource will be fetched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchKeyData {
    /// URL fetched
    pub url: Url,
    /// HTTP method used (GET, POST etc.)
    pub method: Method,
    /// HTTP headers
    pub headers: HeaderMap,
}

impl Hash for FetchKeyData {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        // Header map cannot be hashed directly, so we generate the key and hash that
        let key = self.generate();
        key.hash(state);
    }
}

impl Display for FetchKeyData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.url)
    }
}

impl FetchKeyData {
    /// Creates a new fetch key data with the given URL, method GET and no headers
    pub fn new(url: Url) -> Self {
        Self {
            url,
            method: Method::GET,
            headers: HeaderMap::new(),
        }
    }

    /// Generates a key for coalescing in-flight requests based on the request's method, URL, and headers.
    pub fn generate(&self) -> Option<String> {
        match self.method {
            Method::GET | Method::HEAD => {
                // Only allow safe methods for coalescing
            }
            _ => return None,
        }

        let url = normalize_url(&self.url);
        let h = &self.headers;

        let range = h.get(header::RANGE).and_then(|v| v.to_str().ok()).unwrap_or("");
        let accept = h.get(header::ACCEPT).and_then(|v| v.to_str().ok()).unwrap_or("");
        let accept_enc = h
            .get(header::ACCEPT_ENCODING)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        let accept_lang = h
            .get(header::ACCEPT_LANGUAGE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");

        let auth_hash = h
            .get(header::AUTHORIZATION)
            .map(|v| format!("{:x}", short_hash(v.as_bytes())))
            .unwrap_or_default();

        let cookie_hash = h
            .get(header::COOKIE)
            .map(|v| format!("{:x}", short_hash(v.as_bytes())))
            .unwrap_or_default();

        Some(format!(
            "M={};U={};R={};A={};AL={};AE={};Auth={};C={}",
            self.method, url, range, accept, accept_lang, accept_enc, auth_hash, cookie_hash
        ))
    }
}

// We don't know these types yet
pub(crate) type DocumentId = u64;
pub(crate) type PrefetchId = u64;
pub(crate) type TaskId = u64;

#[derive(Clone)]
pub struct FetchHandle {
    /// Unique ID of this request (for logging and tracking)
    pub req_id: RequestId,
    /// Key data identifying the resource to fetch
    pub key: FetchKeyData,
    /// Cancellation token
    pub cancel: CancellationToken,
}

impl Debug for FetchHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FetchHandle")
            .field("req_id", &self.req_id)
            .field("key", &self.key)
            .field("cancel", &self.cancel)
            .finish()
    }
}

/// A fetch request defines what needs to be fetched, how and where to send the result to
#[derive(Debug, Clone)]
pub struct FetchRequest {
    /// Reference to what initiated this request (navigation, document, prefetch, background task)
    pub reference: RequestReference,
    /// Unique ID of this request (for logging and tracking)
    pub req_id: RequestId,
    /// Key data identifying the resource to fetch
    pub key_data: FetchKeyData,
    /// Priority of this request
    pub priority: Priority,
    /// Who initiated this request
    pub initiator: Initiator,
    /// What kind of resource is being fetched
    pub kind: ResourceKind,
    // whether to stream or buffer
    pub streaming: bool,
    /// Auto decode the request (if for instance, gzipped), or pass directly through to the caller
    pub auto_decode: bool,
    /// Maximum amount of (buffered) bytes we can fetch
    pub max_bytes: Option<usize>,
}

/// FetchResult defines the resource response. Either a stream or buffered response are possible
#[derive(Clone)]
pub enum FetchResult {
    /// Streamed response body
    Stream {
        meta: FetchResultMeta,
        peek_buf: PeekBuf,
        shared: Arc<SharedBody>,
    },
    /// Buffered response body
    Buffered { meta: FetchResultMeta, body: Bytes },
    /// Network error occurred
    Error(NetError),
}

impl FetchResult {
    /// Returns true if the result is an error
    pub fn is_error(&self) -> bool {
        matches!(self, FetchResult::Error(_))
    }

    // Return the metadata if available
    pub fn meta(&self) -> Option<&FetchResultMeta> {
        match self {
            FetchResult::Stream { meta, .. } => Some(meta),
            FetchResult::Buffered { meta, .. } => Some(meta),
            FetchResult::Error(_) => None,
        }
    }
}

impl Debug for FetchResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FetchResult::Stream { meta, .. } => f.debug_struct("FetchResult::Stream").field("meta", meta).finish(),
            FetchResult::Buffered { meta, body } => f
                .debug_struct("FetchResult::Buffered")
                .field("meta", meta)
                .field("body_len", &body.len())
                .finish(),
            FetchResult::Error(e) => f.debug_tuple("FetchResult::Error").field(e).finish(),
        }
    }
}

#[derive(Debug, thiserror::Error, Clone)]
pub enum NetError {
    // Direct reqwest errors
    #[error("net error: reqwest: {0}")]
    Reqwest(#[from] Arc<reqwest::Error>),

    // Redirection errors
    #[error("net error: redirect: {0}")]
    Redirect(Arc<anyhow::Error>),

    // direct I/O errors
    #[error("net error: I/O: {0}")]
    Io(#[from] Arc<std::io::Error>),

    /// Cancelled by the user
    #[error("net error: cancelled: {0}")]
    Cancelled(String),

    // Reading errors
    #[error(transparent)]
    Read(Arc<anyhow::Error>),

    // Any other kind of errors
    #[error(transparent)]
    Other(Arc<anyhow::Error>),

    // Timeout errors
    #[error("net error: timeout: {0}")]
    Timeout(String),
}

impl From<std::io::Error> for NetError {
    fn from(e: std::io::Error) -> Self {
        NetError::Io(Arc::new(e))
    }
}

impl NetError {
    pub fn to_io(&self) -> std::io::Error {
        std::io::Error::other(format!("{self}"))
    }

    pub fn from_anyhow(e: anyhow::Error) -> Self {
        Self::Read(Arc::new(e))
    }
}

/// Defines who initiated the resource load
#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub enum Initiator {
    /// Initiated by the user, UI, or link click
    Navigation,
    /// HTML Parser resource
    Parser,
    /// Initiated by a JS script (or Lua script) (fetch, XHR)
    Script,
    /// CSS @import, font-face
    CSS,
    /// Other undefined type of initiator
    Other,
}

#[cfg(test)]
mod tests {
    use super::*;
    use cow_utils::CowUtils;
    use http::header;
    use tokio::io::AsyncReadExt;

    fn dummy_meta() -> FetchResultMeta {
        FetchResultMeta {
            final_url: Url::parse("https://example.org/").unwrap(),
            status: 200,
            status_text: "OK".into(),
            headers: HeaderMap::new(),
            content_length: None,
            content_type: None,
            has_body: true,
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn bodystream_from_bytes_reads_all() {
        let data = Bytes::from_static(b"hello world");
        let mut s = BodyStream::from_bytes(data.clone());
        assert_eq!(s.len, Some(11));
        assert!(s.is_seekable);
        assert!(s.clonable);

        let mut out = Vec::new();
        s.read_to_end(&mut out).await.unwrap();
        assert_eq!(&out[..], &data[..]);

        // further reads are EOF
        let mut buf = [0u8; 8];
        let n = s.read(&mut buf).await.unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn fetch_key_generate_get_and_headers() {
        let mut fk = FetchKeyData::new(Url::parse("https://example.org/a/b#frag").unwrap());
        // set method GET (already default) and headers that impact the key
        fk.headers.insert(header::RANGE, "bytes=0-99".parse().unwrap());
        fk.headers.insert(header::ACCEPT, "text/html".parse().unwrap());
        fk.headers.insert(header::ACCEPT_LANGUAGE, "en-US".parse().unwrap());
        fk.headers.insert(header::ACCEPT_ENCODING, "gzip".parse().unwrap());
        fk.headers.insert(header::AUTHORIZATION, "Bearer abc".parse().unwrap());
        fk.headers.insert(header::COOKIE, "a=1; b=2".parse().unwrap());

        let key = fk.generate().expect("GET should produce a key");

        // Build the expected string using the same helpers to avoid drift.
        let url_norm = normalize_url(&fk.url);
        let auth_hash = format!("{:x}", short_hash(b"Bearer abc"));
        let cookie_hash = format!("{:x}", short_hash(b"a=1; b=2"));
        let expected = format!(
            "M={};U={};R={};A={};AL={};AE={};Auth={};C={}",
            fk.method, url_norm, "bytes=0-99", "text/html", "en-US", "gzip", auth_hash, cookie_hash
        );

        assert_eq!(key, expected);
        assert!(key.starts_with("M=GET;U=https://example.org/a/b"));
        assert!(!key.contains("#frag")); // fragment must be stripped
    }

    #[test]
    fn fetch_key_generate_post_is_none() {
        let mut fk = FetchKeyData::new(Url::parse("https://example.org/").unwrap());
        fk.method = Method::POST;
        assert!(fk.generate().is_none());
    }

    #[test]
    fn priority_display_is_stable() {
        assert_eq!(format!("{}", Priority::High), "High");
        assert_eq!(format!("{}", Priority::Normal), "Normal");
        assert_eq!(format!("{}", Priority::Low), "Low");
        assert_eq!(format!("{}", Priority::Idle), "Idle");
    }

    #[test]
    fn neterror_helpers_work() {
        // to_io
        let io = NetError::Timeout("oops".into()).to_io();
        assert_eq!(io.kind(), std::io::ErrorKind::Other);
        assert!(io.to_string().cow_to_ascii_lowercase().contains("timeout"));

        // from_anyhow
        let ne = NetError::from_anyhow(anyhow::anyhow!("boom"));
        match ne {
            NetError::Read(_) => {}
            _ => panic!("expected NetError::Read"),
        }
    }

    #[test]
    fn fetchresult_debug_and_clone() {
        let meta = dummy_meta();
        let body = Bytes::from_static(b"DATA");
        let r1 = FetchResult::Buffered {
            meta: meta.clone(),
            body: body.clone(),
        };

        let dbg = format!("{r1:?}");
        assert!(dbg.contains("FetchResult::Buffered"));
        assert!(dbg.contains("body_len: 4"));
        assert!(dbg.contains("status: 200"));

        // clone keeps content
        let r2 = r1.clone();
        match r2 {
            FetchResult::Buffered { meta: m, body: b } => {
                assert_eq!(m.status, 200);
                assert_eq!(&b[..], b"DATA");
            }
            _ => panic!("expected buffered"),
        }
    }

    #[test]
    fn fetchresult_stream_variant_compiles() {
        // just ensure the type is constructible (no runtime path needed here)
        let meta = dummy_meta();
        let shared = Arc::new(SharedBody::new(8));
        let _r = FetchResult::Stream {
            meta,
            peek_buf: PeekBuf::from_slice(b"PEEK"),
            shared,
        };
    }
}

/// The outcome of a main-frame navigation.
#[derive(Debug)]
pub enum ObsoleteNavigationResult {
    /// Main document (e.g. HTML) was parsed successfully.
    Document {
        meta: FetchResultMeta,
        doc: DummyDocument,
    },

    /// A download was started (streaming to file).
    DownloadStarted {
        meta: FetchResultMeta,
        dest: PathBuf,
        // handle: Arc<PumpHandle>,     // result from spawn_pump
    },

    /// A download was already completed (buffered body written directly).
    DownloadFinished {
        meta: FetchResultMeta,
        dest: PathBuf,
    },

    /// Opened externally via a temp file (in-progress).
    OpenExternalStarted {
        meta: FetchResultMeta,
        dest: PathBuf,
        // handle: Arc<PumpHandle>,  // result from spawn_pump
    },

    /// Navigation was cancelled (by user, UA, or engine).
    Cancelled,

    /// Navigation failed for some reason (I/O, parse, etc.).
    Failed {
        meta: Option<FetchResultMeta>,
        error: Arc<anyhow::Error>,
    },
    RenderedByViewer {
        meta: FetchResultMeta,
    },
}
