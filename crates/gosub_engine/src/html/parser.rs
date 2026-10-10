use std::io;

use crate::html::{EngineDocument, RenderConfiguration};
use crate::net::types::{Priority, ResourceKind};
use crate::net::RequestDestination;
use cow_utils::CowUtils;
use gosub_html5::document::builder::DocumentBuilderImpl;
use gosub_html5::parser::Html5Parser;
use gosub_interface::css3::CssSystem;
use gosub_interface::document::Document as _;
use gosub_interface::node::QuirksMode;
use gosub_shared::byte_stream::{ByteStream, Confidence, Encoding};
use gosub_sonar::ReferrerPolicy;
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
    /// The policy the element's `referrerpolicy` attribute names, else the document's where
    /// the element was found: a `<meta name="referrer">` changes it for the elements after it,
    /// not the ones before.
    pub referrer_policy: ReferrerPolicy,
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
    /// Also return the document's source text, for an engine that will hand it
    /// to a renderer process (which re-parses; a DOM cannot cross a fork by
    /// value). Off by default - retaining a copy of every document would tax
    /// engines that render in-process.
    pub capture_source: bool,
    /// The document's referrer policy before any `<meta name="referrer">` in it: the one its
    /// `Referrer-Policy` response header set, else the default.
    pub referrer_policy: ReferrerPolicy,
    /// Set to the document's referrer policy once every `<meta name="referrer">` in it has
    /// been seen, before the parse starts: for what the parse fetches through `stylesheets`.
    pub settled_referrer_policy: Option<std::sync::Arc<std::sync::OnceLock<ReferrerPolicy>>>,
}

impl Default for HtmlParseConfig {
    fn default() -> Self {
        // Matches the `net.document.max_bytes` schema default.
        Self {
            max_bytes: 10 * 1024 * 1024,
            stylesheets: None,
            timing_scope: None,
            capture_source: false,
            referrer_policy: ReferrerPolicy::default(),
            settled_referrer_policy: None,
        }
    }
}

/// Read a document's bytes as source text, without parsing it: what a tab
/// keeps when a renderer process does the parsing. Same size cap and lossy
/// UTF-8 as the captured source of a parsed document.
pub async fn read_document_source<R>(
    base_url: &Url,
    mut reader: R,
    cancel: CancellationToken,
    max_bytes: usize,
) -> Result<std::sync::Arc<str>, DocumentError>
where
    R: AsyncRead + Unpin + Send,
{
    let mut buf: Vec<u8> = Vec::new();
    let mut tmp = [0u8; 16 * 1024];
    loop {
        if cancel.is_cancelled() {
            return Err(DocumentError::Cancelled);
        }
        let n = reader.read(&mut tmp).await?;
        if n == 0 {
            break;
        }
        let remaining = max_bytes.saturating_sub(buf.len()).min(n);
        if remaining > 0 {
            buf.extend_from_slice(&tmp[..remaining]);
        }
        if buf.len() >= max_bytes {
            log::warn!("Document {base_url} exceeds the {max_bytes} byte limit (net.document.max_bytes); truncated");
            let mut drain = [0u8; 16 * 1024];
            while reader.read(&mut drain).await? != 0 {
                if cancel.is_cancelled() {
                    return Err(DocumentError::Cancelled);
                }
            }
            break;
        }
    }
    Ok(std::sync::Arc::<str>::from(String::from_utf8_lossy(&buf).as_ref()))
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
) -> Result<(EngineDocument<C>, Option<std::sync::Arc<str>>, ReferrerPolicy), DocumentError>
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

    // Lossy UTF-8 for the fast resource-discovery regex scan; the parse below
    // decodes properly.
    let html_lossy = String::from_utf8_lossy(&buf);

    // Fire sub-resource callbacks using the fast regex-based scanner so that
    // image/CSS/script fetches are submitted before the full parse completes.
    let policies = ReferrerPolicies::scan(cfg.referrer_policy, &html_lossy);
    let referrer_policy = policies.last();
    if let Some(settled) = &cfg.settled_referrer_policy {
        let _ = settled.set(referrer_policy);
    }
    for hint in discover_resources(&html_lossy, &base_url, &policies) {
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
    // Decoded the way the parse below decodes, so the renderer process re-parses
    // the same text this process would have: a UTF-16 page read as lossy UTF-8
    // would be nothing but replacement characters.
    let source = cfg
        .capture_source
        .then(|| std::sync::Arc::<str>::from(decode_source(&buf, &encoding)));
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
        // A BOM settles the encoding; a chardetng guess stays tentative, so a
        // `<meta charset>` may still change it once.
        if stream.detect_bom().is_some() {
            stream.set_confidence(Confidence::Certain);
        }
        let mut doc = DocumentBuilderImpl::new_document::<C>(Some(base_url));
        let options = gosub_html5::parser::Html5ParserOptions {
            stylesheets,
            ..Default::default()
        };
        let _ = Html5Parser::<C>::parse_document(&mut stream, &mut doc, Some(options));
        let ua = <C::CssSystem as CssSystem>::load_default_useragent_stylesheet();
        doc.add_stylesheet(ua);
        if doc.quirks_mode() == QuirksMode::Quirks {
            if let Some(quirks) = <C::CssSystem as CssSystem>::load_quirks_useragent_stylesheet() {
                doc.add_stylesheet(quirks);
            }
        }

        Ok(doc)
    };

    // No blocking pool on wasm, and no worker to starve either: nothing else was going to
    // run on that thread anyway.
    #[cfg(target_arch = "wasm32")]
    let parsed = parse();
    #[cfg(not(target_arch = "wasm32"))]
    let parsed = match tokio::task::spawn_blocking(parse).await {
        Ok(result) => result,
        // The pool cancels its tasks at runtime shutdown, which is a cancelled
        // navigation by another name; a panic in the parser is not, but there is no
        // document either way.
        Err(e) => {
            log::error!("HTML parse task failed: {e}");
            Err(DocumentError::Cancelled)
        }
    };
    parsed.map(|doc| (doc, source, referrer_policy))
}

/// The referrer policy a response's `Referrer-Policy` header sets (Referrer Policy, "parse a
/// referrer policy from a `Referrer-Policy` header"): the last token naming a policy across
/// every value of the header, `None` when none does.
pub fn header_referrer_policy(headers: &http::HeaderMap) -> Option<ReferrerPolicy> {
    let values: Vec<&str> = headers
        .get_all("referrer-policy")
        .iter()
        .filter_map(|value| value.to_str().ok())
        .collect();
    ReferrerPolicy::parse_header(&values.join(","))
}

/// A document's referrer policy once every `<meta name="referrer">` in `html` has been
/// processed, starting from `initial`, the response header's.
pub fn document_referrer_policy(initial: ReferrerPolicy, html: &str) -> ReferrerPolicy {
    ReferrerPolicies::scan(initial, html).last()
}

/// Where in a document its referrer policy changes: the start, then each
/// `<meta name="referrer">` whose content names a policy (HTML, "Standard metadata names",
/// `referrer`). A fetch for an element uses the policy in effect where the element sits.
struct ReferrerPolicies {
    initial: ReferrerPolicy,
    /// Byte offset of each meta that set a policy, in document order.
    changes: Vec<(usize, ReferrerPolicy)>,
}

impl ReferrerPolicies {
    fn scan(initial: ReferrerPolicy, html: &str) -> Self {
        let changes = RE_META
            .iter()
            .flat_map(|re| re.find_iter(html))
            .filter_map(|tag| meta_referrer_policy(tag.as_str()).map(|policy| (tag.start(), policy)))
            .collect();
        Self { initial, changes }
    }

    fn at(&self, offset: usize) -> ReferrerPolicy {
        self.changes
            .iter()
            .take_while(|(start, _)| *start < offset)
            .last()
            .map_or(self.initial, |(_, policy)| *policy)
    }

    fn last(&self) -> ReferrerPolicy {
        self.changes.last().map_or(self.initial, |(_, policy)| *policy)
    }
}

/// The policy a `<meta>` tag sets, if it is `name="referrer"` with a content naming one.
///
/// The content is lowercased and matched whole, the four legacy keywords mapped first; an
/// empty or unknown value changes nothing.
fn meta_referrer_policy(tag: &str) -> Option<ReferrerPolicy> {
    if !tag_attribute(tag, "name")?.eq_ignore_ascii_case("referrer") {
        return None;
    }
    let value = tag_attribute(tag, "content")?.cow_to_ascii_lowercase();
    let value = match value.as_ref() {
        "never" => "no-referrer",
        "default" => "no-referrer-when-downgrade",
        "always" => "unsafe-url",
        "origin-when-crossorigin" => "origin-when-cross-origin",
        other => other,
    };
    policy_token(value)
}

/// The value of attribute `name` in a raw start `tag`, unquoted. The first of a repeated
/// attribute is the one the tokenizer keeps.
fn tag_attribute<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    RE_ATTR
        .iter()
        .flat_map(|re| re.captures_iter(tag))
        .find(|cap| {
            cap.name("key")
                .is_some_and(|key| key.as_str().eq_ignore_ascii_case(name))
        })
        .and_then(|cap| cap.name("value"))
        .map(|value| unquote(value.as_str()))
}

/// The policy an element's `referrerpolicy` attribute in a raw start `tag` names, if any.
fn tag_referrer_policy(tag: &str) -> Option<ReferrerPolicy> {
    tag_attribute(tag, "referrerpolicy").and_then(policy_token)
}

/// A referrer policy written in markup: the whole value, ASCII case-insensitive, as an
/// enumerated attribute is matched. `None` for an unknown, empty or padded value, which
/// leaves the policy as it was (`parse_token` would trim the padding away).
pub(crate) fn policy_token(value: &str) -> Option<ReferrerPolicy> {
    if value.trim() != value {
        return None;
    }
    ReferrerPolicy::parse_token(value)
}

/// The document's text as the parser reads it: UTF-16 when the detection said so,
/// UTF-8 otherwise (the only other encoding the parser decodes), minus a BOM.
fn decode_source(bytes: &[u8], encoding: &Encoding) -> String {
    let utf16 = |bytes: &[u8], unit: fn([u8; 2]) -> u16| -> String {
        let bytes = bytes
            .strip_prefix(&[0xFF, 0xFE])
            .or_else(|| bytes.strip_prefix(&[0xFE, 0xFF]))
            .unwrap_or(bytes);
        // A trailing odd byte is dropped, as `from_utf16_lossy` could not use it anyway.
        let units: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|pair| unit(*pair)).collect();
        String::from_utf16_lossy(&units)
    };
    match encoding {
        Encoding::UTF16LE => utf16(bytes, u16::from_le_bytes),
        Encoding::UTF16BE => utf16(bytes, u16::from_be_bytes),
        _ => String::from_utf8_lossy(bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes)).into_owned(),
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

/// Compile a literal regex pattern, or `None` if it will not compile.
///
/// These patterns are literals in this file, so a failure here is a typo in this repository
/// rather than anything a page can cause - but this is a *prescan* that produces preload hints,
/// so a pattern that does not compile costs the hints it would have found and nothing else.
/// It used to be an `unwrap()`, which paid for a typo with the process.
fn re(pattern: &str) -> Option<Regex> {
    match Regex::new(pattern) {
        Ok(regex) => Some(regex),
        Err(e) => {
            log::error!("resource prescan pattern {pattern:?} did not compile, so it will find nothing: {e}");
            None
        }
    }
}

static RE_LINK_STYLESHEET: Lazy<Option<Regex>> = Lazy::new(|| {
    // allow "..." or '...' or unquoted; capture into the *same* group `href`
    re(
        r#"(?is)<\s*link\b[^>]*\brel\s*=\s*(?:"stylesheet"|'stylesheet')[^>]*\bhref\s*=\s*(?P<href>"[^"]*"|'[^']*'|[^\s>]+)[^>]*>"#,
    )
});

static RE_SCRIPT_SRC: Lazy<Option<Regex>> =
    Lazy::new(|| re(r#"(?is)<\s*script\b[^>]*\bsrc\s*=\s*(?P<src>"[^"]*"|'[^']*'|[^\s>]+)[^>]*>"#));

static RE_META: Lazy<Option<Regex>> = Lazy::new(|| re(r#"(?is)<\s*meta\b[^>]*>"#));

static RE_ATTR: Lazy<Option<Regex>> =
    Lazy::new(|| re(r#"(?is)\s(?P<key>[a-z-]+)\s*=\s*(?P<value>"[^"]*"|'[^']*'|[^\s>]+)"#));

static RE_ASYNC_ATTR: Lazy<Option<Regex>> = Lazy::new(|| re(r#"\basync\b"#));

static RE_DEFER_ATTR: Lazy<Option<Regex>> = Lazy::new(|| re(r#"\bdefer\b"#));

static RE_IMG_SRC: Lazy<Option<Regex>> =
    Lazy::new(|| re(r#"(?is)<\s*img\b[^>]*\bsrc\s*=\s*(?P<src>"[^"]*"|'[^']*'|[^\s>]+)[^>]*>"#));

fn discover_resources(html: &str, base: &Url, policies: &ReferrerPolicies) -> Vec<ResourceHint> {
    let mut out = Vec::new();

    // Stylesheets
    for cap in RE_LINK_STYLESHEET.iter().flat_map(|re| re.captures_iter(html)) {
        let Some(m) = cap.name("href") else {
            continue;
        };
        let Ok(u) = resolve(base, unquote(m.as_str())) else {
            continue;
        };
        let tag = cap.get(0).map_or("", |m| m.as_str());
        out.push(ResourceHint {
            url: u,
            referrer_policy: tag_referrer_policy(tag).unwrap_or_else(|| policies.at(m.start())),
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
    for cap in RE_SCRIPT_SRC.iter().flat_map(|re| re.captures_iter(html)) {
        let tag = cap.get(0).map_or("", |m| m.as_str());
        let tag_lower = tag.cow_to_ascii_lowercase();
        // A script is blocking unless it has async or defer attributes
        // A pattern that did not compile cannot say the script is async or deferred, so the
        // script is treated as blocking - the conservative answer, and the HTML default.
        let has = |re: &Lazy<Option<Regex>>| re.as_ref().is_some_and(|re| re.is_match(tag_lower.as_ref()));
        let blocking = !has(&RE_ASYNC_ATTR) && !has(&RE_DEFER_ATTR);
        let Some(m) = cap.name("src") else {
            continue;
        };
        let Ok(u) = resolve(base, unquote(m.as_str())) else {
            continue;
        };
        out.push(ResourceHint {
            url: u,
            referrer_policy: tag_referrer_policy(tag).unwrap_or_else(|| policies.at(m.start())),
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
    for cap in RE_IMG_SRC.iter().flat_map(|re| re.captures_iter(html)) {
        let Some(m) = cap.name("src") else {
            continue;
        };
        let Ok(u) = resolve(base, unquote(m.as_str())) else {
            continue;
        };
        let tag = cap.get(0).map_or("", |m| m.as_str());
        out.push(ResourceHint {
            url: u,
            referrer_policy: tag_referrer_policy(tag).unwrap_or_else(|| policies.at(m.start())),
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

    /// The source handed to a renderer process is the text the parser read, BOM
    /// and all encodings it knows accounted for.
    #[test]
    fn the_captured_source_is_decoded_like_the_parse() {
        let text = "<p>héllo</p>";
        let mut le = vec![0xFF, 0xFE];
        le.extend(text.encode_utf16().flat_map(u16::to_le_bytes));
        let mut be = vec![0xFE, 0xFF];
        be.extend(text.encode_utf16().flat_map(u16::to_be_bytes));
        let mut utf8 = b"\xEF\xBB\xBF".to_vec();
        utf8.extend_from_slice(text.as_bytes());

        assert_eq!(decode_source(&le, &Encoding::UTF16LE), text);
        assert_eq!(decode_source(&be, &Encoding::UTF16BE), text);
        assert_eq!(decode_source(&utf8, &Encoding::UTF8), text);
    }

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

        let (_doc, _, _) = parse_main_document_stream::<DefaultRenderConfig, _, _>(
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

    /// HN-style markup: no doctype, `<center><table>`. The spec's quirks-mode table rules
    /// keep the cells from inheriting the centering; a standards-mode document gets no such sheet.
    #[tokio::test(flavor = "current_thread")]
    async fn quirks_mode_documents_get_the_quirks_useragent_sheet() {
        let body = "<center><table><tr><td>row</td></tr></table></center>";
        let base = Url::parse("https://example.com/").unwrap();
        let parse = |html: String| {
            parse_main_document_stream::<DefaultRenderConfig, _, _>(
                base.clone(),
                reader_from_str(&html),
                CancellationToken::new(),
                HtmlParseConfig::default(),
                |_| {},
            )
        };
        let parse = |html: String| async { parse(html).await.map(|(doc, _source, _policy)| doc) };

        let quirks = parse(format!("<html><body>{body}</body></html>")).await.unwrap();
        assert_eq!(quirks.quirks_mode(), QuirksMode::Quirks);
        let standards = parse(format!("<!DOCTYPE html><html><body>{body}</body></html>"))
            .await
            .unwrap();
        assert_eq!(standards.quirks_mode(), QuirksMode::NoQuirks);

        assert_eq!(
            quirks.stylesheets().len(),
            standards.stylesheets().len() + 1,
            "quirks documents carry exactly one extra user-agent sheet"
        );
        assert!(
            quirks.stylesheets().iter().any(|s| s.url.contains("useragent-quirks")),
            "the extra sheet is the quirks sheet"
        );
    }

    /// A `<meta name="referrer">` changes the policy for the elements after it, and the
    /// document's policy ends up as the last one that named a policy.
    #[tokio::test(flavor = "current_thread")]
    async fn a_meta_referrer_applies_from_where_it_stands() {
        let html = r#"<html><head>
            <img src="before.png">
            <meta name="Referrer" content="NO-REFERRER">
            <img src="after.png">
            <meta name="referrer" content="bogus">
            <meta content="origin" name="referrer">
            <img src="last.png">
        </head></html>"#;
        let mut hints = Vec::new();
        let (_doc, _, policy) = parse_main_document_stream::<DefaultRenderConfig, _, _>(
            Url::parse("https://example.com/").unwrap(),
            reader_from_str(html),
            CancellationToken::new(),
            HtmlParseConfig {
                referrer_policy: ReferrerPolicy::SameOrigin,
                ..Default::default()
            },
            |h| hints.push(h),
        )
        .await
        .unwrap();
        let policy_of = |name: &str| {
            hints
                .iter()
                .find(|h| h.url.path() == format!("/{name}"))
                .map(|h| h.referrer_policy)
        };
        assert_eq!(policy_of("before.png"), Some(ReferrerPolicy::SameOrigin));
        assert_eq!(policy_of("after.png"), Some(ReferrerPolicy::NoReferrer));
        assert_eq!(policy_of("last.png"), Some(ReferrerPolicy::Origin));
        assert_eq!(policy, ReferrerPolicy::Origin);
    }

    /// An element's own `referrerpolicy` beats the document's, wherever a meta left it.
    #[test]
    fn an_elements_referrerpolicy_beats_the_documents() {
        let html = r#"<meta name="referrer" content="no-referrer">
            <img src="/plain.png">
            <img referrerpolicy="unsafe-url" src="/own.png">
            <script src="/s.js" referrerpolicy="origin"></script>
            <link rel="stylesheet" referrerpolicy="same-origin" href="/s.css">
            <img src="/bogus.png" referrerpolicy="never">"#;
        let base = Url::parse("https://example.com/").unwrap();
        let hints = discover_resources(html, &base, &ReferrerPolicies::scan(ReferrerPolicy::default(), html));
        let policy_of = |path: &str| hints.iter().find(|h| h.url.path() == path).map(|h| h.referrer_policy);
        assert_eq!(policy_of("/plain.png"), Some(ReferrerPolicy::NoReferrer));
        assert_eq!(policy_of("/own.png"), Some(ReferrerPolicy::UnsafeUrl));
        assert_eq!(policy_of("/s.js"), Some(ReferrerPolicy::Origin));
        assert_eq!(policy_of("/s.css"), Some(ReferrerPolicy::SameOrigin));
        assert_eq!(
            policy_of("/bogus.png"),
            Some(ReferrerPolicy::NoReferrer),
            "legacy keywords are meta-only"
        );
    }

    /// The legacy keywords map to their policies; an unknown, padded or empty content, or a
    /// meta of another name, changes nothing.
    #[test]
    fn meta_referrer_content() {
        let policy = |tag: &str| document_referrer_policy(ReferrerPolicy::UnsafeUrl, tag);
        assert_eq!(
            policy(r#"<meta name=referrer content=never>"#),
            ReferrerPolicy::NoReferrer
        );
        assert_eq!(
            policy(r#"<meta name=referrer content=default>"#),
            ReferrerPolicy::NoReferrerWhenDowngrade
        );
        assert_eq!(
            policy(r#"<meta name=referrer content=always>"#),
            ReferrerPolicy::UnsafeUrl
        );
        assert_eq!(
            policy(r#"<meta name='referrer' content='origin-when-crossorigin'>"#),
            ReferrerPolicy::OriginWhenCrossOrigin
        );
        let unchanged = [
            r#"<meta name=referrer content="">"#,
            r#"<meta name=referrer content=" origin">"#,
            r#"<meta name=referrer content="origin, no-referrer">"#,
            r#"<meta name=description content=origin>"#,
            r#"<meta content=origin>"#,
        ];
        for tag in unchanged {
            assert_eq!(policy(tag), ReferrerPolicy::UnsafeUrl, "{tag}");
        }
    }

    /// The header's last token naming a policy wins, across every value it was sent with.
    #[test]
    fn referrer_policy_header() {
        let mut headers = http::HeaderMap::new();
        assert_eq!(header_referrer_policy(&headers), None);
        headers.append("referrer-policy", "no-referrer, bogus".parse().unwrap());
        assert_eq!(header_referrer_policy(&headers), Some(ReferrerPolicy::NoReferrer));
        headers.append("referrer-policy", "same-origin".parse().unwrap());
        assert_eq!(header_referrer_policy(&headers), Some(ReferrerPolicy::SameOrigin));
        headers.append("referrer-policy", "unknown".parse().unwrap());
        assert_eq!(header_referrer_policy(&headers), Some(ReferrerPolicy::SameOrigin));
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
