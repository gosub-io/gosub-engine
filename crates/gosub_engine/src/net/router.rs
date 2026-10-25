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
        /// The document's referrer policy (see [`ParsedDocument::into_parts`]).
        referrer_policy: gosub_sonar::ReferrerPolicy,
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
    /// Made at the response headers: `spool` is the body still being written to a temp file,
    /// so the embedder hears of the download while it arrives. A body whose declared length
    /// is over the spool cap fails the navigation instead, and there is no fallback to a
    /// fresh fetch. See [`TabCommand::StartDownload`](crate::events::TabCommand::StartDownload)
    /// for why the body is captured rather than re-requested. Boxed to keep the variants of
    /// comparable size.
    DownloadOffer {
        meta: Box<crate::net::types::FetchResultMeta>,
        spool: Spool,
    },
}

/// The disk held by download spools across every tab of an engine.
///
/// A spool's temp file lives as long as its offer: an embedder that never answers keeps it,
/// and each tab keeps up to 8. `net.download.max_spool_bytes` bounds one file and the 8
/// bound a tab, but nothing bounded the tabs together, so a page could fill the temp
/// filesystem by opening more of them. Every spool now takes its bytes from this budget,
/// and gives them back when its file goes.
#[derive(Debug, Default)]
pub(crate) struct SpoolBudget {
    used: std::sync::atomic::AtomicU64,
}

impl SpoolBudget {
    /// Bytes the spools hold now.
    pub(crate) fn used(&self) -> u64 {
        self.used.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Whether `bytes` more would go over `limit` (0 = no limit).
    fn would_exceed(&self, bytes: u64, limit: u64) -> bool {
        limit > 0 && self.used().saturating_add(bytes) > limit
    }
}

/// What one spool holds of a [`SpoolBudget`]. Grows as the body is written and gives
/// everything back when dropped, so it has to stay with the temp file.
#[derive(Debug)]
pub(crate) struct SpoolLease {
    budget: Arc<SpoolBudget>,
    held: u64,
}

impl SpoolLease {
    fn new(budget: Arc<SpoolBudget>) -> Self {
        Self { budget, held: 0 }
    }

    /// Take `bytes` more, unless that goes over `limit` (0 = no limit).
    fn grow(&mut self, bytes: u64, limit: u64) -> bool {
        use std::sync::atomic::Ordering;
        let mut used = self.budget.used.load(Ordering::Acquire);
        loop {
            let next = used.saturating_add(bytes);
            if limit > 0 && next > limit {
                return false;
            }
            match self
                .budget
                .used
                .compare_exchange_weak(used, next, Ordering::AcqRel, Ordering::Acquire)
            {
                Ok(_) => {
                    self.held += bytes;
                    return true;
                }
                Err(now) => used = now,
            }
        }
    }
}

impl Drop for SpoolLease {
    fn drop(&mut self) {
        self.budget
            .used
            .fetch_sub(self.held, std::sync::atomic::Ordering::AcqRel);
    }
}

/// The bounds on a download spool, read per navigation.
#[derive(Debug, Clone)]
pub(crate) struct SpoolLimits {
    /// `net.download.max_spool_bytes`: one body. 0 = no limit.
    pub(crate) per_download: u64,
    /// `net.download.max_total_spool_bytes`: every spool of the engine together. 0 = no limit.
    pub(crate) total: u64,
    /// The engine's spools, which `total` bounds.
    pub(crate) budget: Arc<SpoolBudget>,
}

impl SpoolLimits {
    /// No bounds, and a budget of its own.
    #[cfg(test)]
    pub(crate) fn unlimited() -> Self {
        Self {
            per_download: 0,
            total: 0,
            budget: Arc::default(),
        }
    }
}

/// A download body on disk, and its share of the [`SpoolBudget`]: the bytes are given back
/// when this is dropped, or once [`into_path`](Self::into_path)'s path goes.
#[derive(Debug)]
pub(crate) struct SpooledFile {
    path: tempfile::TempPath,
    lease: SpoolLease,
}

impl SpooledFile {
    /// The file, and what keeps it counted: hold the lease for as long as the file is in the
    /// temp directory.
    pub(crate) fn into_parts(self) -> (tempfile::TempPath, SpoolLease) {
        (self.path, self.lease)
    }
}

impl AsRef<std::path::Path> for SpooledFile {
    fn as_ref(&self) -> &std::path::Path {
        &self.path
    }
}

impl std::ops::Deref for SpooledFile {
    type Target = std::path::Path;
    fn deref(&self) -> &std::path::Path {
        &self.path
    }
}

/// How far a [`Spool`] has got.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SpoolProgress {
    /// This many bytes are in the temp file so far.
    Writing(u64),
    /// The whole body, this many bytes, is in the temp file.
    Done(u64),
    /// Writing stopped with this error; the temp file is gone.
    Failed(String),
}

/// A download body being written to a temp file in the background, so the offer can go to
/// the embedder at the response headers rather than once the whole body has arrived.
///
/// Dropping a `Spool` before [`finish`](Self::finish) stops the writing and removes the file:
/// an offer that is evicted, or a tab that closes, leaves nothing behind.
pub(crate) struct Spool {
    progress: tokio::sync::watch::Receiver<SpoolProgress>,
    result: tokio::sync::oneshot::Receiver<anyhow::Result<SpooledFile>>,
    stop: tokio_util::sync::DropGuard,
}

impl std::fmt::Debug for Spool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Spool")
            .field("progress", &*self.progress.borrow())
            .finish()
    }
}

impl Spool {
    /// Start writing `peek_buf` and then `body` to a new temp file, on a task of its own.
    fn start(body: BodyContent, peek_buf: PeekBuf, limits: SpoolLimits) -> Self {
        let (progress_tx, progress) = tokio::sync::watch::channel(SpoolProgress::Writing(0));
        let (result_tx, result) = tokio::sync::oneshot::channel();
        let stop = tokio_util::sync::CancellationToken::new();
        let stopped = stop.clone();
        crate::util::spawn_named("download-spool", async move {
            let on_written = |n: u64| {
                progress_tx.send_replace(SpoolProgress::Writing(n));
            };
            let written = tokio::select! {
                r = spool_body_to_temp(body, peek_buf, &limits, &on_written) => r,
                _ = stopped.cancelled() => return,
            };
            progress_tx.send_replace(match &written {
                Ok((_, n)) => SpoolProgress::Done(*n),
                Err(e) => SpoolProgress::Failed(format!("{e:#}")),
            });
            let _ = result_tx.send(written.map(|(path, _)| path));
        });
        Self {
            progress,
            result,
            stop: stop.drop_guard(),
        }
    }

    /// Follows the writing: the latest [`SpoolProgress`], and a wakeup when it moves.
    pub(crate) fn progress(&self) -> tokio::sync::watch::Receiver<SpoolProgress> {
        self.progress.clone()
    }

    /// Wait for the whole body and take its temp file. Dropping this future before it
    /// completes stops the writing, as dropping the `Spool` would.
    pub(crate) async fn finish(self) -> anyhow::Result<SpooledFile> {
        let Self { result, stop, .. } = self;
        let written = result
            .await
            .unwrap_or_else(|_| Err(anyhow!("the download stopped before its body was written")));
        stop.disarm();
        written
    }
}

/// Write a response body to a temp file and return it and its length, calling
/// `on_written` with the running total as it goes.
///
/// Streamed, not buffered: an offer can be gigabytes and the embedder may never answer.
/// The file is a [`tempfile::TempPath`], removed if the offer is dropped, and every byte of
/// it is taken from the engine's [`SpoolBudget`] before it is written.
async fn spool_body_to_temp(
    body: BodyContent,
    peek_buf: PeekBuf,
    limits: &SpoolLimits,
    on_written: &(dyn Fn(u64) + Send + Sync),
) -> anyhow::Result<(SpooledFile, u64)> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    // An unaccepted offer occupies the temp filesystem; refuse to spool past the caps.
    let max_bytes = limits.per_download;
    let over = |n: u64| max_bytes > 0 && n > max_bytes;
    let too_large = || anyhow!("download exceeds net.download.max_spool_bytes ({max_bytes} bytes)");
    let mut lease = SpoolLease::new(Arc::clone(&limits.budget));
    let mut take = |n: u64| {
        if lease.grow(n, limits.total) {
            Ok(())
        } else {
            Err(over_total(limits.total))
        }
    };

    let path = tempfile::Builder::new()
        .prefix("gosub-download-")
        .tempfile()
        .context("create download temp file")?
        .into_temp_path();

    let mut file = tokio::fs::File::create(&path)
        .await
        .with_context(|| format!("open {}", path.display()))?;

    let written = match body {
        BodyContent::Buffered { body } => {
            if over(body.len() as u64) {
                return Err(too_large());
            }
            take(body.len() as u64)?;
            file.write_all(&body).await.context("write download temp file")?;
            body.len() as u64
        }
        BodyContent::Stream { shared } => {
            // Lossless: writing to disk is slower than the network, and a lagging reader of a
            // shared body is dropped; this one makes the body wait for it instead.
            let mut reader = SharedBody::combined_reader_lossless(peek_buf, shared);
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
                take(n as u64)?;
                file.write_all(&buf[..n]).await.context("write download temp file")?;
                on_written(total);
            }
            total
        }
    };
    file.flush().await.context("flush download temp file")?;
    Ok((SpooledFile { path, lease }, written))
}

/// The error for a download that would take the engine's spools past `total` bytes.
fn over_total(total: u64) -> anyhow::Error {
    anyhow!("downloads waiting for an answer would hold more than net.download.max_total_spool_bytes ({total} bytes)")
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
        "<!DOCTYPE html><html><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"></head>\
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
        "<!DOCTYPE html><html><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"></head>\
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
                let (doc, source, referrer_policy) = parsed.into_parts();
                Ok(RoutedOutcome::MainDocument {
                    doc: doc.map(Arc::new),
                    source,
                    referrer_policy,
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
                let (doc, source, referrer_policy) = hooks
                    .html
                    .parse_bytes(request, handle, meta, html.as_bytes())
                    .await?
                    .into_parts();
                return Ok(RoutedOutcome::MainDocument {
                    doc: doc.map(Arc::new),
                    source,
                    referrer_policy,
                });
            }
            // Binary content or an explicit attachment: offer it to the embedder as a
            // download instead of failing the navigation. The offer goes now, at the headers;
            // the body is spooled to disk behind it, while we still have it - see
            // `DownloadOffer`. A body that says it will not fit fails the navigation here:
            // an offer without its body would have to re-request the URL on accept, which is
            // exactly what spooling exists to avoid.
            let limits = hooks.download_spool.clone();
            let max = limits.per_download;
            if let Some(len) = meta.content_length.filter(|&len| max > 0 && len > max) {
                return Err(anyhow!(
                    "download of {len} bytes exceeds net.download.max_spool_bytes ({max} bytes)"
                ));
            }
            if meta
                .content_length
                .is_some_and(|len| limits.budget.would_exceed(len, limits.total))
            {
                return Err(over_total(limits.total));
            }
            Ok(RoutedOutcome::DownloadOffer {
                meta: Box::new(meta),
                spool: Spool::start(body_content, peek_buf, limits),
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A spool hands over its temp file once the whole body is written, and says so.
    #[tokio::test]
    async fn a_spool_finishes_with_the_whole_body() {
        let shared = Arc::new(SharedBody::new(8));
        let spool = Spool::start(
            BodyContent::Stream { shared: shared.clone() },
            PeekBuf::from_slice(b"head-"),
            SpoolLimits::unlimited(),
        );
        shared.push(Bytes::from_static(b"body"));
        shared.finish();
        let mut progress = spool.progress();
        let path = spool.finish().await.expect("spooled");
        assert_eq!(std::fs::read(&*path).unwrap(), b"head-body");
        let _ = progress.changed().await;
        assert_eq!(*progress.borrow(), SpoolProgress::Done(9));
    }

    /// A lease takes from the budget up to its limit, and gives back all it took when dropped.
    #[test]
    fn a_spool_lease_is_bounded_and_given_back() {
        let budget = Arc::new(SpoolBudget::default());
        let mut a = SpoolLease::new(Arc::clone(&budget));
        let mut b = SpoolLease::new(Arc::clone(&budget));
        assert!(a.grow(60, 100));
        assert!(!b.grow(50, 100), "60 + 50 is over the limit");
        assert!(b.grow(40, 100));
        assert_eq!(budget.used(), 100);
        assert!(budget.would_exceed(1, 100));
        drop(a);
        assert_eq!(budget.used(), 40);
        assert!(b.grow(1_000, 0), "0 is no limit");
        drop(b);
        assert_eq!(budget.used(), 0);
    }

    /// A spooled file keeps its bytes counted until it is dropped, after the writing is done.
    #[tokio::test]
    async fn a_spooled_file_holds_its_bytes_until_dropped() {
        let limits = SpoolLimits::unlimited();
        let spool = Spool::start(
            BodyContent::Buffered {
                body: Bytes::from_static(b"twelve bytes"),
            },
            PeekBuf::empty(),
            limits.clone(),
        );
        let file = spool.finish().await.expect("spooled");
        assert_eq!(limits.budget.used(), 12);
        drop(file);
        assert_eq!(limits.budget.used(), 0);
    }

    /// A body that would take the spools past the total fails, and leaves nothing counted.
    #[tokio::test]
    async fn a_spool_past_the_total_fails() {
        let limits = SpoolLimits {
            total: 8,
            ..SpoolLimits::unlimited()
        };
        let spool = Spool::start(
            BodyContent::Buffered {
                body: Bytes::from_static(b"twelve bytes"),
            },
            PeekBuf::empty(),
            limits.clone(),
        );
        let error = spool.finish().await.expect_err("over the total");
        assert!(
            format!("{error:#}").contains("net.download.max_total_spool_bytes"),
            "got {error:#}"
        );
        assert_eq!(limits.budget.used(), 0);
    }

    /// Dropping a spool whose body is still arriving stops the writing: an offer that is
    /// evicted or a tab that closes leaves no task reading on.
    #[tokio::test]
    async fn dropping_a_spool_stops_its_writing() {
        let shared = Arc::new(SharedBody::new(8));
        let spool = Spool::start(
            BodyContent::Stream { shared: shared.clone() },
            PeekBuf::empty(),
            SpoolLimits::unlimited(),
        );
        shared.push(Bytes::from_static(b"part"));
        let mut progress = spool.progress();
        drop(spool);
        // The writer ends, and with it the progress it reported.
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while progress.changed().await.is_ok() {}
        })
        .await
        .expect("the spool kept writing after it was dropped");
    }
}
