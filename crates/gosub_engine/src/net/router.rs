use crate::engine::resource_pipeline::css::DummyStylesheet;
use crate::engine::resource_pipeline::font::DummyFont;
use crate::engine::resource_pipeline::js::DummyJsDocument;
use crate::engine::resource_pipeline::ResourcePipelines;
use crate::engine::types::PeekBuf;
use crate::engine::UaPolicy;
use crate::html::{EngineDocument, RenderConfiguration};
use crate::net::decision::types::BlockReason;
use crate::net::decision::ResponseClass;
use crate::net::types::{FetchHandle, FetchRequest, FetchResult};
use crate::net::{decide_handling, stream_to_bytes, HandlingDecision, RenderTarget, RequestDestination, SharedBody};
use anyhow::{anyhow, Context};
use bytes::Bytes;
use std::sync::Arc;

/// The outcome of routing a fetch result.
///
/// The subresource payloads below are produced here and discarded by the tab worker -
/// placeholders until the async resource pipeline consumes them.
#[derive(Debug)]
#[allow(dead_code)]
pub enum RoutedOutcome<C: RenderConfiguration> {
    /// The main document has been parsed and is ready. The second field is
    /// the document's source text, captured when the engine renders
    /// out-of-process (its renderer re-parses; a DOM cannot cross a fork).
    MainDocument {
        /// `None` when the renderer process parses instead of this one.
        doc: Option<Arc<EngineDocument<C>>>,
        source: Option<Arc<str>>,
    },
    /// The resource has been rendered in a viewer (text, image, pdf, etc.).
    ViewerRendered(Bytes),

    /// A stylesheet has been loaded and parsed.
    CssLoaded(DummyStylesheet),
    /// A script has been loaded and executed.
    ScriptExecuted(DummyJsDocument),
    /// A font has been loaded.
    FontLoaded(DummyFont),

    /// The request was blocked (with reason).
    Blocked(BlockReason),

    /// A top-level navigation to non-renderable content: offer it as a download.
    ///
    /// `spooled` is the temp file the body was written to; `None` if spooling failed, and the
    /// download falls back to a fresh fetch. See [`TabCommand::StartDownload`](crate::events::TabCommand::StartDownload)
    /// for why the body is captured rather than re-requested. Boxed to keep the variants of
    /// comparable size.
    DownloadOffer {
        meta: Box<crate::net::types::FetchResultMeta>,
        spooled: tempfile::TempPath,
    },
}

/// Write a response body to a temp file and return its path.
///
/// Streamed, not buffered: an offer can be gigabytes and the embedder may never answer.
/// Returns a [`tempfile::TempPath`] so the file is removed if the offer is dropped.
async fn spool_body_to_temp(
    body: BodyContent,
    peek_buf: PeekBuf,
    max_bytes: u64,
) -> anyhow::Result<tempfile::TempPath> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    // An unaccepted offer occupies the temp filesystem; refuse to spool past the cap.
    let over = |n: u64| max_bytes > 0 && n > max_bytes;
    let too_large = || anyhow!("download exceeds net.download.max_spool_bytes ({max_bytes} bytes)");

    let path = tempfile::Builder::new()
        .prefix("gosub-download-")
        .tempfile()
        .context("create download temp file")?
        .into_temp_path();

    let mut file = tokio::fs::File::create(&path)
        .await
        .with_context(|| format!("open {}", path.display()))?;

    match body {
        BodyContent::Buffered { body } => {
            if over(body.len() as u64) {
                return Err(too_large());
            }
            file.write_all(&body).await.context("write download temp file")?;
        }
        BodyContent::Stream { shared } => {
            let mut reader = SharedBody::combined_reader(peek_buf, shared);
            let mut buf = vec![0u8; 64 * 1024];
            let mut total: u64 = 0;
            loop {
                let n = reader.read(&mut buf).await.context("read download body")?;
                if n == 0 {
                    break;
                }
                total += n as u64;
                if over(total) {
                    return Err(too_large());
                }
                file.write_all(&buf[..n]).await.context("write download temp file")?;
            }
        }
    }
    file.flush().await.context("flush download temp file")?;
    Ok(path)
}

fn html_escape(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len() + 16);
    for ch in text.chars() {
        match ch {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            _ => escaped.push(ch),
        }
    }
    escaped
}

/// Minimal HTML document presenting `body` as escaped plain text, used to render
/// textual non-HTML navigations (text/plain, unparseable JSON) the way mainstream
/// browsers do.
fn text_document_html(body: &[u8]) -> String {
    let escaped = html_escape(&String::from_utf8_lossy(body));
    format!(
        "<!DOCTYPE html><html><head><meta charset=\"utf-8\"></head>\
         <body><pre style=\"white-space: pre-wrap; word-wrap: break-word;\">{escaped}</pre></body></html>"
    )
}

/// Beyond these the viewer shows escaped text instead: it is a convenience,
/// not a reason to hold a large parse tree in the broker.
const JSON_VIEWER_MAX_BYTES: usize = 8 * 1024 * 1024;
const JSON_VIEWER_MAX_DEPTH: usize = 64;

// JSON viewer palette (GitHub-light-ish).
const JSON_KEY_COLOR: &str = "#6f42c1";
const JSON_STR_COLOR: &str = "#22863a";
const JSON_NUM_COLOR: &str = "#005cc5";
const JSON_LIT_COLOR: &str = "#d73a49";

fn json_scalar_html(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::Null => format!("<span style=\"color:{JSON_LIT_COLOR}\">null</span>"),
        serde_json::Value::Bool(b) => format!("<span style=\"color:{JSON_LIT_COLOR}\">{b}</span>"),
        serde_json::Value::Number(n) => format!("<span style=\"color:{JSON_NUM_COLOR}\">{n}</span>"),
        serde_json::Value::String(s) => {
            format!("<span style=\"color:{JSON_STR_COLOR}\">\"{}\"</span>", html_escape(s))
        }
        // Arrays and objects are rendered by `json_lines`, which never hands one to this
        // function. Showing the value rather than asserting that means a devtools panel
        // shows something odd instead of taking the process with it.
        other => format!(
            "<span style=\"color:{JSON_LIT_COLOR}\">{}</span>",
            html_escape(&other.to_string())
        ),
    }
}

/// Emit one `(indent, html)` pair per pretty-printed line of `v`.
fn json_lines(v: &serde_json::Value, depth: usize, key: Option<&str>, trail: &str, out: &mut Vec<(usize, String)>) {
    let prefix = key
        .map(|k| format!("<span style=\"color:{JSON_KEY_COLOR}\">\"{}\"</span>: ", html_escape(k)))
        .unwrap_or_default();
    // Past this the subtree is shown compact: the recursion runs on a task stack.
    if depth > JSON_VIEWER_MAX_DEPTH && !v.is_null() && (v.is_object() || v.is_array()) {
        out.push((depth, format!("{prefix}{}{trail}", html_escape(&v.to_string()))));
        return;
    }
    match v {
        serde_json::Value::Object(map) if map.is_empty() => out.push((depth, format!("{prefix}{{}}{trail}"))),
        serde_json::Value::Array(arr) if arr.is_empty() => out.push((depth, format!("{prefix}[]{trail}"))),
        serde_json::Value::Object(map) => {
            out.push((depth, format!("{prefix}{{")));
            let n = map.len();
            for (i, (k, val)) in map.iter().enumerate() {
                json_lines(val, depth + 1, Some(k), if i + 1 < n { "," } else { "" }, out);
            }
            out.push((depth, format!("}}{trail}")));
        }
        serde_json::Value::Array(arr) => {
            out.push((depth, format!("{prefix}[")));
            let n = arr.len();
            for (i, val) in arr.iter().enumerate() {
                json_lines(val, depth + 1, None, if i + 1 < n { "," } else { "" }, out);
            }
            out.push((depth, format!("]{trail}")));
        }
        _ => out.push((depth, format!("{prefix}{}{trail}", json_scalar_html(v)))),
    }
}

/// JSON viewer document: pretty-printed and syntax-highlighted, like the built-in
/// viewers in mainstream browsers.
///
/// Rendered as one `<div>` per line — block boxes force line breaks — because the
/// layout engine does not implement `white-space: pre` yet; indentation is inline
/// `padding-left`. Once `white-space` lands this can become a single `<pre>`.
fn json_document_html(value: &serde_json::Value) -> String {
    let mut lines = Vec::new();
    json_lines(value, 0, None, "", &mut lines);
    let mut body = String::with_capacity(lines.len() * 48);
    for (indent, html) in lines {
        if indent > 0 {
            body.push_str(&format!("<div style=\"padding-left:{}em\">{html}</div>", indent * 2));
        } else {
            body.push_str(&format!("<div>{html}</div>"));
        }
    }
    format!(
        "<!DOCTYPE html><html><head><meta charset=\"utf-8\"></head>\
         <body style=\"font-family:monospace;font-size:13px;color:#24292e;background:#ffffff\">{body}</body></html>"
    )
}

enum BodyContent {
    Stream { shared: Arc<SharedBody> },
    Buffered { body: Bytes },
}

impl BodyContent {
    // Collect into bytes; a streamed body is re-joined with its peek buffer.
    #[allow(clippy::wrong_self_convention)]
    async fn to_bytes(self, peek_buf: PeekBuf) -> anyhow::Result<Bytes> {
        match self {
            BodyContent::Stream { shared } => {
                let buf = stream_to_bytes(peek_buf.clone(), shared).await?;
                Ok(buf)
            }
            BodyContent::Buffered { body } => Ok(body),
        }
    }
}

/// Route a fetch result based on its destination and the UA policy.
pub async fn route_response_for<C: RenderConfiguration>(
    dest: RequestDestination,
    handle: FetchHandle,
    request: FetchRequest,
    fetch_result: FetchResult,
    policy: &UaPolicy,
    hooks: &mut ResourcePipelines<C>,
) -> anyhow::Result<RoutedOutcome<C>> {
    let (meta, body_content, peek_buf) = match fetch_result {
        FetchResult::Stream { meta, peek_buf, shared } => (meta, BodyContent::Stream { shared }, peek_buf),
        FetchResult::Buffered { meta, body } => {
            let peek_len = body.len().min(5 * 1024);
            let peek_buf = PeekBuf::from_slice(&body[0..peek_len]);
            (meta, BodyContent::Buffered { body }, peek_buf)
        }
        FetchResult::Error(e) => {
            return Err(anyhow!(e));
        }
    };

    let outcome = decide_handling(&meta, dest, peek_buf.clone(), policy);

    match (dest, outcome.decision, body_content) {
        (RequestDestination::Document, HandlingDecision::Render(target), body_content) => match target {
            RenderTarget::HtmlParser => {
                let parsed = match body_content {
                    BodyContent::Stream { shared } => {
                        hooks.html.parse_stream(request, handle, meta, peek_buf, shared).await?
                    }
                    BodyContent::Buffered { body } => {
                        hooks.html.parse_bytes(request, handle, meta, body.as_ref()).await?
                    }
                };
                let (doc, source) = parsed.into_parts();
                Ok(RoutedOutcome::MainDocument {
                    doc: doc.map(Arc::new),
                    source,
                })
            }
            RenderTarget::CssParser => Ok(RoutedOutcome::ViewerRendered(body_content.to_bytes(peek_buf).await?)),
            RenderTarget::JsEngine => Ok(RoutedOutcome::ViewerRendered(body_content.to_bytes(peek_buf).await?)),
            RenderTarget::ImageDecoder => Ok(RoutedOutcome::ViewerRendered(body_content.to_bytes(peek_buf).await?)),
            RenderTarget::FontLoader => Ok(RoutedOutcome::ViewerRendered(body_content.to_bytes(peek_buf).await?)),
            RenderTarget::PdfViewer => Ok(RoutedOutcome::ViewerRendered(body_content.to_bytes(peek_buf).await?)),
        },
        (RequestDestination::Document, HandlingDecision::Download { .. }, body_content) => {
            // A top-level navigation to a textual non-HTML resource (a JSON API
            // endpoint, text/plain, …) renders as plain text, like mainstream
            // browsers. Explicit attachments and binary content stay unloadable
            // until downloads are implemented.
            if !outcome.disposition_attachment && matches!(outcome.class, ResponseClass::Json | ResponseClass::Text) {
                let body = body_content.to_bytes(peek_buf).await?;
                // JSON gets the highlighted viewer; anything unparseable (and
                // text/plain) falls back to escaped plain text.
                let html = if outcome.class == ResponseClass::Json && body.len() <= JSON_VIEWER_MAX_BYTES {
                    match serde_json::from_slice::<serde_json::Value>(&body) {
                        Ok(value) => json_document_html(&value),
                        Err(_) => text_document_html(&body),
                    }
                } else {
                    text_document_html(&body)
                };
                let mut meta = meta;
                meta.content_type = Some("text/html; charset=utf-8".into());
                // The synthesized viewer page is a document like any other: its
                // source travels along so a renderer process can re-parse it too.
                let (doc, source) = hooks
                    .html
                    .parse_bytes(request, handle, meta, html.as_bytes())
                    .await?
                    .into_parts();
                return Ok(RoutedOutcome::MainDocument {
                    doc: doc.map(Arc::new),
                    source,
                });
            }
            // Binary content or an explicit attachment: offer it to the embedder as a
            // download instead of failing the navigation. The body is spooled to disk now,
            // while we still have it - see `DownloadOffer`.
            // If the body cannot be captured the navigation fails: an offer without its body
            // would have to re-request the URL on accept, which is exactly what spooling exists
            // to avoid.
            let spooled = spool_body_to_temp(body_content, peek_buf, hooks.max_download_spool_bytes).await?;
            Ok(RoutedOutcome::DownloadOffer {
                meta: Box::new(meta),
                spooled,
            })
        }

        // -------- Sub resources (no UA prompts) --------
        (RequestDestination::Style, HandlingDecision::Render(RenderTarget::CssParser), body_content) => {
            let stylesheet = match body_content {
                BodyContent::Stream { shared } => hooks.css.parse_stream(meta, peek_buf, shared).await?,
                BodyContent::Buffered { body } => hooks.css.parse_bytes(meta, body.as_ref()).await?,
            };
            Ok(RoutedOutcome::CssLoaded(stylesheet))
        }
        (RequestDestination::Script, HandlingDecision::Render(RenderTarget::JsEngine), body_content) => {
            let script = match body_content {
                BodyContent::Stream { shared } => hooks.js.parse_stream(meta, peek_buf, shared).await?,
                BodyContent::Buffered { body } => hooks.js.parse_bytes(meta, body.as_ref()).await?,
            };
            Ok(RoutedOutcome::ScriptExecuted(script))
        }
        // Images are decoded where they are painted (a renderer, or the media
        // store through the decoder process), never by this router.
        (RequestDestination::Image, HandlingDecision::Render(RenderTarget::ImageDecoder), _) => {
            Err(anyhow::anyhow!("images are not decoded by the resource router"))
        }
        (RequestDestination::Font, HandlingDecision::Render(RenderTarget::FontLoader), body_content) => {
            let font = match body_content {
                BodyContent::Stream { shared } => hooks.fonts.parse_stream(meta, peek_buf, shared).await?,
                BodyContent::Buffered { body } => hooks.fonts.parse_bytes(meta, body.as_ref()).await?,
            };
            Ok(RoutedOutcome::FontLoaded(font))
        }

        // Safety net: any other subresource decision (Download, or a Render target that
        // doesn't match the destination) is treated as a policy block.
        (_, HandlingDecision::Download { .. } | HandlingDecision::Render(_), _) => {
            Ok(RoutedOutcome::Blocked(BlockReason::Policy))
        }
    }
}
