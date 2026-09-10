use std::io;

use crate::html::{EngineDocument, RenderConfiguration};
use crate::net::types::{Priority, ResourceKind};
use crate::net::RequestDestination;
use cow_utils::CowUtils;
use gosub_html5::document::builder::DocumentBuilderImpl;
use gosub_html5::parser::Html5Parser;
use gosub_interface::css3::CssSystem;
use gosub_interface::document::Document as _;
use gosub_shared::byte_stream::{ByteStream, Encoding};
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
    pub rel: Option<String>, // e.g. "stylesheet"
    /// The attribute we discovered this from.
    pub from_attr: &'static str, // e.g. "href" or "src"
    /// The referrer URL if applicable.
    pub referrer: Option<Url>,
    /// Whether this is a cross-origin request.
    pub cross_origin: bool,
    /// The integrity attribute value if applicable.
    pub integrity: Option<String>,
    /// Suggested fetch priority.
    pub priority: Priority,
}

/// Errors from buffering and parsing a main document stream.
#[derive(thiserror::Error, Debug)]
pub enum DocumentError {
    /// I/O error while reading the document stream.
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),

    /// URL parsing error
    #[error("URL error: {0}")]
    Url(#[from] url::ParseError),

    /// Cancellation (navigation cancelled).
    #[error("Cancelled")]
    Cancelled,
}

/// Configuration for parsing a main document (see [`parse_main_document_stream`]).
#[derive(Debug, Clone)]
pub struct HtmlParseConfig {
    /// Max bytes to buffer from the stream; a larger document is truncated (with a warning).
    /// The engine reads this from the `net.document.max_bytes` setting.
    pub max_bytes: usize,
    /// Where a blocking script's stylesheets come from.
    ///
    /// A classic `<script>` may not run until the sheets before it have applied, and the
    /// parser no longer fetches them, so it asks this instead and waits. `None` means no
    /// script waits for anything and the sheets are resolved after the parse.
    pub stylesheets: Option<std::sync::Arc<dyn gosub_html5::parser::StylesheetSource>>,

    /// Navigation these timings belong to, if this parse is part of one.
    ///
    /// Entered around the synchronous parse below, which is where `decode.html` and the
    /// parser's own blocking `net.fetch.css` are recorded. `None` for parses with no
    /// navigation behind them; those samples stay unattributed rather than misfiled.
    pub timing_scope: Option<gosub_shared::timing::ScopeId>,
}

impl Default for HtmlParseConfig {
    fn default() -> Self {
        // Matches the `net.document.max_bytes` schema default.
        Self {
            max_bytes: 10 * 1024 * 1024,
            stylesheets: None,
            timing_scope: None,
        }
    }
}

/// Main entry point: buffer the HTML stream, parse it into a real DOM document,
/// and report discovered sub-resources.
///
/// - `base_url`: used to resolve relative URLs and as the document URL.
/// - `reader`: the response body stream (after the UA has chosen Render).
/// - `cancel`: cancellation token (tab/nav cancellation).
/// - `cfg`: buffer limit config.
/// - `on_discover`: callback invoked for each sub-resource hint found.
pub async fn parse_main_document_stream<C, R, F>(
    base_url: Url,
    mut reader: R,
    cancel: CancellationToken,
    cfg: HtmlParseConfig,
    mut on_discover: F,
) -> Result<EngineDocument<C>, DocumentError>
where
    C: RenderConfiguration,
    R: AsyncRead + Unpin + Send + 'static,
    F: FnMut(ResourceHint) + Send,
{
    // Buffer the full stream (up to cfg.max_bytes); bail on cancellation.
    let mut buf = Vec::with_capacity(32 * 1024);
    let mut tmp = [0u8; 16 * 1024];

    loop {
        if cancel.is_cancelled() {
            return Err(DocumentError::Cancelled);
        }

        let n = reader.read(&mut tmp).await?;
        if n == 0 {
            break;
        }

        let remaining = cfg.max_bytes.saturating_sub(buf.len()).min(n);
        if remaining > 0 {
            buf.extend_from_slice(&tmp[..remaining]);
        }
        // If we hit the cap, we still drain the stream to EOF quickly
        // to avoid keeping the connection open unnecessarily.
        if buf.len() >= cfg.max_bytes {
            log::warn!(
                "Document {base_url} exceeds the {} byte limit (net.document.max_bytes); parsing truncated content",
                cfg.max_bytes
            );
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

    // Use lossy UTF-8 only for the fast resource-discovery regex scan.
    let html_lossy = String::from_utf8_lossy(&buf);

    // Fire sub-resource callbacks using the fast regex-based scanner so that
    // image/CSS/script fetches are submitted before the full parse completes.
    for hint in discover_resources(&html_lossy, &base_url) {
        on_discover(hint);
    }

    // Detect encoding from the raw bytes (BOM check + chardetng), then build a
    // properly-decoded stream.  We cannot call set_encoding() on an Unknown-
    // encoded stream because tell_bytes() returns buffer.len() when chars is
    // empty, which would advance the position to EOF.
    let encoding = {
        let mut tmp = ByteStream::new(Encoding::Unknown, None);
        tmp.read_from_bytes(&buf)?;
        tmp.detect_encoding()
    };
    // The parse below is synchronous, and because the parser fetches external stylesheets
    // inline it can sit still for as long as a server cares to stay silent. Run on a
    // runtime worker, that starves every task the worker owns -- and always at least one:
    // tokio parks the most recently spawned task in a slot no other worker may steal from,
    // so the last subresource this document just discovered is never polled. A page with
    // three stylesheets fetched two, and the third was the one still missing when the
    // parser went looking for it. So the parse goes to the blocking pool, where a thread
    // is allowed to sit still.
    //
    // The timing scope is a thread-local, so it is entered inside the closure -- on the
    // thread that actually does the work. It covers `decode.html` and the `net.fetch.css`
    // samples the parser's own blocking fetches produce.
    let timing_scope = cfg.timing_scope;
    let stylesheets = cfg.stylesheets.clone();
    let parse = move || -> Result<EngineDocument<C>, DocumentError> {
        let _scope = timing_scope.map(gosub_shared::timing::enter_scope);

        let mut stream = ByteStream::new(encoding, None);
        stream.read_from_bytes(&buf)?;
        let mut doc = DocumentBuilderImpl::new_document::<C>(Some(base_url));
        let options = gosub_html5::parser::Html5ParserOptions {
            stylesheets,
            ..Default::default()
        };
        let _ = Html5Parser::<C>::parse_document(&mut stream, &mut doc, Some(options));
        let ua = <C::CssSystem as CssSystem>::load_default_useragent_stylesheet();
        doc.add_stylesheet(ua);

        Ok(doc)
    };

    // No blocking pool on wasm, and no worker to starve either: nothing else was going to
    // run on that thread anyway.
    #[cfg(target_arch = "wasm32")]
    {
        parse()
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        match tokio::task::spawn_blocking(parse).await {
            Ok(result) => result,
            // The pool cancels its tasks at runtime shutdown, which is a cancelled
            // navigation by another name; a panic in the parser is not, but there is no
            // document either way.
            Err(e) => {
                log::error!("HTML parse task failed: {e}");
                Err(DocumentError::Cancelled)
            }
        }
    }
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

/// Compile a literal regex pattern.
fn re(pattern: &str) -> Regex {
    #[allow(clippy::unwrap_used)] // PANIC-SAFE: all callers pass literal patterns, exercised by tests
    Regex::new(pattern).unwrap()
}

static RE_LINK_STYLESHEET: Lazy<Regex> = Lazy::new(|| {
    // allow "..." or '...' or unquoted; capture into the *same* group `href`
    re(
        r#"(?is)<\s*link\b[^>]*\brel\s*=\s*(?:"stylesheet"|'stylesheet')[^>]*\bhref\s*=\s*(?P<href>"[^"]*"|'[^']*'|[^\s>]+)[^>]*>"#,
    )
});

static RE_SCRIPT_SRC: Lazy<Regex> =
    Lazy::new(|| re(r#"(?is)<\s*script\b[^>]*\bsrc\s*=\s*(?P<src>"[^"]*"|'[^']*'|[^\s>]+)[^>]*>"#));

static RE_ASYNC_ATTR: Lazy<Regex> = Lazy::new(|| re(r#"\basync\b"#));

static RE_DEFER_ATTR: Lazy<Regex> = Lazy::new(|| re(r#"\bdefer\b"#));

static RE_IMG_SRC: Lazy<Regex> =
    Lazy::new(|| re(r#"(?is)<\s*img\b[^>]*\bsrc\s*=\s*(?P<src>"[^"]*"|'[^']*'|[^\s>]+)[^>]*>"#));

fn discover_resources(html: &str, base: &Url) -> Vec<ResourceHint> {
    let mut out = Vec::new();

    // Stylesheets
    for cap in RE_LINK_STYLESHEET.captures_iter(html) {
        let Some(m) = cap.name("href") else {
            continue;
        };
        let Ok(u) = resolve(base, unquote(m.as_str())) else {
            continue;
        };
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

    // Scripts
    for cap in RE_SCRIPT_SRC.captures_iter(html) {
        let tag = cap.get(0).map_or("", |m| m.as_str());
        let tag_lower = tag.cow_to_ascii_lowercase();
        // A script is blocking unless it has async or defer attributes
        let blocking = !RE_ASYNC_ATTR.is_match(tag_lower.as_ref()) && !RE_DEFER_ATTR.is_match(tag_lower.as_ref());
        let Some(m) = cap.name("src") else {
            continue;
        };
        let Ok(u) = resolve(base, unquote(m.as_str())) else {
            continue;
        };
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

    // Images
    for cap in RE_IMG_SRC.captures_iter(html) {
        let Some(m) = cap.name("src") else {
            continue;
        };
        let Ok(u) = resolve(base, unquote(m.as_str())) else {
            continue;
        };
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

    out
}

fn resolve(base: &Url, candidate: &str) -> Result<Url, url::ParseError> {
    // Tolerate whitespace, no-op fragments, etc.
    let trimmed = candidate.trim();
    if trimmed.is_empty() {
        return Err(url::ParseError::EmptyHost);
    }
    base.join(&decode_ampersands(trimmed))
}

/// Turn escaped ampersands back into `&`.
///
/// This scanner reads raw HTML, so an attribute arrives exactly as it was written -- and HTML
/// requires an ampersand in an attribute to be escaped. A URL with a query string therefore
/// shows up as `load.php?lang=en&amp;only=scripts`, and fetching that literally asks the
/// server for a parameter called `amp;only`. Wikipedia answers with a couple of hundred bytes
/// of nothing, which is not an error anyone notices until they read the bytes.
///
/// Only the ampersand forms are decoded, not the full character-reference grammar. Every
/// other reference is either invalid in a URL or already percent-encoded, and a partial
/// decoder that pretended otherwise would be its own source of wrong URLs. The DOM path is
/// unaffected: it gets properly decoded attribute values from the tokenizer.
fn decode_ampersands(url: &str) -> std::borrow::Cow<'_, str> {
    if !url.contains('&') {
        return std::borrow::Cow::Borrowed(url);
    }
    use cow_utils::CowUtils;
    let decoded = url
        .cow_replace("&amp;", "&")
        .cow_replace("&AMP;", "&")
        .cow_replace("&#38;", "&")
        .cow_replace("&#x26;", "&")
        .cow_replace("&#X26;", "&")
        .into_owned();
    if decoded == url {
        std::borrow::Cow::Borrowed(url)
    } else {
        std::borrow::Cow::Owned(decoded)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_escaped_ampersand_does_not_reach_the_network() {
        let base = Url::parse("https://en.wikipedia.org/wiki/BASIC").unwrap();

        // As it appears in real markup: HTML requires the ampersand to be escaped.
        let resolved = resolve(&base, "/w/load.php?lang=en&amp;only=scripts").unwrap();
        assert_eq!(
            resolved.as_str(),
            "https://en.wikipedia.org/w/load.php?lang=en&only=scripts"
        );
        assert_eq!(
            resolved.query_pairs().count(),
            2,
            "two parameters, not one called amp;only"
        );

        // Numeric forms too.
        assert_eq!(
            resolve(&base, "/a?x=1&#38;y=2").unwrap().as_str(),
            "https://en.wikipedia.org/a?x=1&y=2"
        );
        assert_eq!(
            resolve(&base, "/a?x=1&#x26;y=2").unwrap().as_str(),
            "https://en.wikipedia.org/a?x=1&y=2"
        );

        // A bare ampersand is already what it should be, and is left alone.
        assert_eq!(
            resolve(&base, "/a?x=1&y=2").unwrap().as_str(),
            "https://en.wikipedia.org/a?x=1&y=2"
        );
    }
    use crate::html::DefaultRenderConfig;
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

        parse_main_document_stream::<DefaultRenderConfig, _, _>(
            base.clone(),
            reader_from_str(html),
            cancel,
            HtmlParseConfig::default(),
            |h| hints.push(h),
        )
        .await
        .unwrap();

        // Ensure we discovered 3 resources with resolved URLs
        assert_eq!(hints.len(), 3);
        assert!(hints
            .iter()
            .any(|h| h.kind == ResourceKind::Stylesheet && h.url.as_str() == "https://example.com/style.css"));
        assert!(hints.iter().any(|h| h.kind == ResourceKind::Script { blocking: true }
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

        let res = parse_main_document_stream::<DefaultRenderConfig, _, _>(
            base,
            reader,
            cancel,
            HtmlParseConfig::default(),
            |_h| {},
        )
        .await;

        match res {
            Err(DocumentError::Cancelled) => {}
            other => panic!("expected Cancelled, got {:?}", other),
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn truncates_at_max_bytes() {
        let base = Url::parse("https://e.test/").unwrap();
        let big = "A".repeat(150_000); // 150 KiB
        let cfg = HtmlParseConfig {
            max_bytes: 64 * 1024, // 64 KiB
            ..Default::default()
        };

        // Just verify truncated input still produces a valid document (no panic).
        parse_main_document_stream::<DefaultRenderConfig, _, _>(
            base,
            reader_from_str(&big),
            CancellationToken::new(),
            cfg,
            |_h| {},
        )
        .await
        .unwrap();
    }
}
