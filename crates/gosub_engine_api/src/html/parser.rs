use std::io;

use crate::net::types::{Priority, ResourceKind};
use crate::net::RequestDestination;
use once_cell::sync::Lazy;
use regex::Regex;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio_util::sync::CancellationToken;
use url::Url;

/// A hint to the engine/IO layer that a subresource should be fetched.
#[derive(Debug, Clone)]
pub struct ResourceHint {
    /// Absolute URL of the resource to fetch.
    pub url: Url,
    /// The destination type (affects request headers, etc).
    pub dest: RequestDestination,
    /// The kind of resource (affects priority, etc).
    pub kind: ResourceKind,
    /// The `rel` attribute value if applicable.
    pub rel: Option<String>,     // e.g. "stylesheet"
    /// The attribute we discovered this from.
    pub from_attr: &'static str, // e.g. "href" or "src
    /// The referrer URL if applicable.
    pub referrer: Option<Url>,
    /// Whether this is a cross-origin request.
    pub cross_origin: bool,
    /// The integrity attribute value if applicable.
    pub integrity: Option<String>,
    /// Suggested fetch priority.
    pub priority: Priority,
}

/// A dummy document structure that is a placeholder for an actual DOM document.
#[derive(Debug, Clone, PartialEq)]
pub struct DummyDocument {
    /// The final URL of the document (after redirects).
    pub final_url: Url,
    /// The document title, if any.
    pub title: Option<String>,
    /// Whole HTML as UTF-8 (best-effort).
    pub raw_html: String,
}

impl DummyDocument {
    /// Synthesize a dummy document from a string.
    pub fn from(html: String, final_url: Url) -> Self {
        let title = discover_title(&html);
        Self {
            final_url,
            title,
            raw_html: html,
        }
    }
}

/// Error type for this dummy parser.
#[derive(thiserror::Error, Debug)]
pub enum DocumentError {
    /// UTF-8 error
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),

    /// URL parsing error
    #[error("URL error: {0}")]
    Url(#[from] url::ParseError),

    /// Cancellation (navigation cancelled).
    #[error("Cancelled")]
    Cancelled,
}

/// Configuration of the dummy HTML5 parser.
#[derive(Debug, Clone)]
pub struct DummyHtml5Config {
    /// Max bytes to buffer from the stream. We read the entire stream up to this limit.
    pub max_bytes: usize,
}

impl Default for DummyHtml5Config {
    fn default() -> Self {
        Self {
            max_bytes: 1 * 1024 * 1024,
        } // 1 MiB
    }
}

/// Main entry point: read stream, synthesize a doc, and report discovered sub resources.
///
/// - `base_url`: used to resolve relative URLs.
/// - `reader`: the response body stream (already after UA has chosen Render).
/// - `cancel`: cancellation token (tab/nav cancellation).
/// - `on_discover`: callback invoked for each resource we find (enqueue fetch from here).
pub async fn parse_main_document_stream<R, F>(
    base_url: Url,
    mut reader: R,
    cancel: CancellationToken,
    cfg: DummyHtml5Config,
    mut on_discover: F,
) -> Result<DummyDocument, DocumentError>
where
    R: AsyncRead + Unpin + Send + 'static,
    F: FnMut(ResourceHint) + Send,
{
    // Read the stream into a bounded buffer; bail if cancelled.
    let mut buf = Vec::with_capacity(32 * 1024);
    let mut tmp = [0u8; 16 * 1024];

    loop {
        // Check cancellation before each read.
        if cancel.is_cancelled() {
            return Err(DocumentError::Cancelled);
        }

        // Read a chunk
        let n = reader.read(&mut tmp).await?;
        if n == 0 {
            // Eof encountered
            break;
        }

        let remaining = cfg.max_bytes.saturating_sub(buf.len()).min(n);
        if remaining > 0 {
            buf.extend_from_slice(&tmp[..remaining]);
        }
        // If we hit the cap, we still drain the stream to EOF quickly
        // to avoid keeping the connection open unnecessarily.
        if buf.len() >= cfg.max_bytes {
            // Drain (non-blocking-ish) without growing memory
            // We don't strictly need to, but it's polite to the transport.
            let mut drain = [0u8; 16 * 1024];
            while reader.read(&mut drain).await? != 0 {
                if cancel.is_cancelled() {
                    return Err(DocumentError::Cancelled);
                }
            }
            break;
        }
    }

    // Best-effort UTF-8 for discovery and title extraction.
    let html = String::from_utf8_lossy(&buf).into_owned();

    // Discover resources (css/js/img) and fire callbacks.
    for hint in discover_resources(&html, &base_url) {
        on_discover(hint);
    }

    // Synthesize a document (optionally extract <title>…</title>).
    let title = discover_title(&html);

    Ok(DummyDocument {
        final_url: base_url,
        title,
        raw_html: html,
    })
}

// ======== Forgiving resource discovery (regex-based) ========
fn unquote(s: &str) -> &str {
    let b = s.as_bytes();
    if b.len() >= 2 && ((b[0] == b'"' && b[b.len() - 1] == b'"') || (b[0] == b'\'' && b[b.len() - 1] == b'\'')) {
        &s[1..s.len() - 1]
    } else {
        s
    }
}

static RE_LINK_STYLESHEET: Lazy<Regex> = Lazy::new(|| {
    // allow "..." or '...' or unquoted; capture into the *same* group `href`
    Regex::new(
        r#"(?is)<\s*link\b[^>]*\brel\s*=\s*(?:"stylesheet"|'stylesheet')[^>]*\bhref\s*=\s*(?P<href>"[^"]*"|'[^']*'|[^\s>]+)[^>]*>"#
    ).unwrap()
});

static RE_SCRIPT_SRC: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"(?is)<\s*script\b[^>]*\bsrc\s*=\s*(?P<src>"[^"]*"|'[^']*'|[^\s>]+)[^>]*>"#).unwrap());

static RE_ASYNC_ATTR: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"\basync\b"#).unwrap());

static RE_DEFER_ATTR: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"\bdefer\b"#).unwrap());

static RE_IMG_SRC: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"(?is)<\s*img\b[^>]*\bsrc\s*=\s*(?P<src>"[^"]*"|'[^']*'|[^\s>]+)[^>]*>"#).unwrap());

static RE_TITLE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"(?is)<\s*title\s*>\s*(?P<title>.*?)\s*<\s*/\s*title\s*>"#).unwrap());

fn discover_title(html: &str) -> Option<String> {
    RE_TITLE
        .captures(html)
        .and_then(|c| c.name("title").map(|m| m.as_str().trim().to_string()))
}

fn discover_resources(html: &str, base: &Url) -> Vec<ResourceHint> {
    let mut out = Vec::new();

    // Stylesheets
    for cap in RE_LINK_STYLESHEET.captures_iter(html) {
        if let Some(m) = cap.name("href") {
            if let Ok(u) = resolve(base, unquote(m.as_str())) {
                out.push(ResourceHint {
                    url: u,
                    dest: RequestDestination::Document,
                    referrer: None,
                    cross_origin: false,
                    integrity: None,
                    kind: ResourceKind::Stylesheet,
                    rel: Some("stylesheet".to_string()),
                    from_attr: "href",
                    priority: Priority::High,
                });
            }
        }
    }

    // Scripts
    for cap in RE_SCRIPT_SRC.captures_iter(html) {
        let tag = cap.get(0).map_or("", |m| m.as_str());
        let tag_lower = tag.to_ascii_lowercase();
        // A script is blocking unless it has async or defer attributes
        let blocking = !RE_ASYNC_ATTR.is_match(&tag_lower) && !RE_DEFER_ATTR.is_match(&tag_lower);
        if let Some(m) = cap.name("src") {
            if let Ok(u) = resolve(base, unquote(m.as_str())) {
                out.push(ResourceHint {
                    url: u,
                    kind: ResourceKind::Script { blocking },
                    rel: None,
                    from_attr: "src",
                    dest: RequestDestination::Script,
                    referrer: None,
                    cross_origin: false,
                    integrity: None,
                    priority: Priority::Normal,
                });
            }
        }
    }

    // Images
    for cap in RE_IMG_SRC.captures_iter(html) {
        if let Some(m) = cap.name("src") {
            if let Ok(u) = resolve(base, unquote(m.as_str())) {
                out.push(ResourceHint {
                    url: u,
                    kind: ResourceKind::Image,
                    rel: None,
                    from_attr: "src",
                    dest: RequestDestination::Image,
                    referrer: None,
                    cross_origin: false,
                    integrity: None,
                    priority: Priority::Low,
                });
            }
        }
    }

    out
}

fn resolve(base: &Url, candidate: &str) -> Result<Url, url::ParseError> {
    // Tolerate whitespace, no-op fragments, etc.
    let trimmed = candidate.trim();
    if trimmed.is_empty() {
        return Err(url::ParseError::EmptyHost);
    }
    base.join(trimmed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use futures::stream;
    use tokio_util::io::StreamReader;

    fn reader_from_str(s: &str) -> impl AsyncRead + Unpin + Send + 'static {
        // One-chunk stream -> AsyncRead
        let it = stream::iter(vec![Ok::<Bytes, io::Error>(Bytes::from(s.to_owned()))]);
        StreamReader::new(it)
    }

    #[tokio::test(flavor = "current_thread")]
    async fn parses_title_and_discovers_resources() {
        let html = r#"
            <html>
              <head>
                <title> Hello World </title>
                <link rel="stylesheet" href="/style.css">
              </head>
              <body>
                <script src="app.js"></script>
                <img src="images/logo.png">
              </body>
            </html>
        "#;

        let base = Url::parse("https://example.com/path/index.html").unwrap();
        let cancel = CancellationToken::new();
        let mut hints = Vec::new();

        let doc = parse_main_document_stream(
            base.clone(),
            reader_from_str(html),
            cancel,
            DummyHtml5Config::default(),
            |h| hints.push(h),
        )
        .await
        .unwrap();

        assert_eq!(doc.title.as_deref(), Some("Hello World"));
        assert!(doc.raw_html.contains("Hello World"));

        // Ensure we discovered 3 resources with resolved URLs
        assert_eq!(hints.len(), 3);
        dbg!(&hints);
        assert!(hints
            .iter()
            .any(|h| h.kind == ResourceKind::Stylesheet && h.url.as_str() == "https://example.com/style.css"));
        assert!(hints
            .iter()
            .any(|h| h.kind == ResourceKind::Script { blocking: true }
                && h.url.as_str() == "https://example.com/path/app.js"));
        assert!(hints
            .iter()
            .any(|h| h.kind == ResourceKind::Image && h.url.as_str() == "https://example.com/path/images/logo.png"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn honors_cancellation() {
        let base = Url::parse("https://e.test/").unwrap();

        // Make a stream that hangs so we can cancel before read completes.
        use futures::stream::pending;
        let pending_stream = pending::<Result<Bytes, io::Error>>();
        let reader = StreamReader::new(pending_stream);

        let cancel = CancellationToken::new();
        cancel.cancel(); // cancel immediately

        let res = parse_main_document_stream(base, reader, cancel, DummyHtml5Config::default(), |_h| {}).await;

        match res {
            Err(DocumentError::Cancelled) => {}
            other => panic!("expected Cancelled, got {:?}", other),
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn truncates_at_max_bytes() {
        let base = Url::parse("https://e.test/").unwrap();
        let big = "A".repeat(150_000); // 150 KiB
        let cfg = DummyHtml5Config { max_bytes: 64 * 1024 }; // 64 KiB

        let doc = parse_main_document_stream(
            base,
            reader_from_str(&big),
            CancellationToken::new(),
            cfg.clone(),
            |_h| {},
        )
        .await
        .unwrap();

        assert_eq!(doc.raw_html.len(), cfg.max_bytes);
    }

    #[test]
    fn discover_title_basic() {
        assert_eq!(discover_title("<title>x</title>").as_deref(), Some("x"));
        assert_eq!(
            discover_title("<TITLE>  spaced \n</TITLE>").as_deref(),
            Some("spaced")
        );
        assert_eq!(discover_title("<head></head>").is_none(), true);
    }
}
