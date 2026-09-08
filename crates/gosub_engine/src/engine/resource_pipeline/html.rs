use crate::engine::types::{IoChannel, PeekBuf, RequestId};
use crate::html::{parse_main_document_stream, EngineDocument, RenderConfiguration, ResourceHint};
use crate::net::req_ref_tracker::REF_REGISTRY;
use crate::net::types::{FetchHandle, FetchRequest, FetchResult, FetchResultMeta, Initiator};
use crate::net::{submit_to_io, SharedBody};
use crate::util::spawn_named;
use crate::zone::ZoneId;
use anyhow::anyhow;
use async_trait::async_trait;
use bytes::Bytes;
use futures_util::stream;
use gosub_shared::timing_guard;
use http::Method;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::io::AsyncRead;
use tokio::task::JoinHandle;
use tokio_util::io::StreamReader;

/// One `oneshot` per stylesheet this parse discovered, keyed by URL.
type SheetBodies = Arc<Mutex<HashMap<String, tokio::sync::oneshot::Receiver<SheetBody>>>>;

/// A fetched stylesheet: its `Content-Type` and its bytes. `None` when it could not be had.
type SheetBody = Option<(Option<String>, Vec<u8>)>;

/// Hand a fetched stylesheet to the parse waiting for it, or tell the global hand-off what
/// became of a resource somebody else may be waiting on. Exactly one of the two applies,
/// which is why they share a function: every fetch has to end in one or the other, or a
/// consumer waits for bytes that are never coming.
fn deliver(sheet_tx: Option<tokio::sync::oneshot::Sender<SheetBody>>, url: &url::Url, body: SheetBody) {
    match sheet_tx {
        // The receiver is gone when the parse ended without wanting this sheet after all
        // (a cancelled navigation, or a `<link>` the scanner saw and the parser did not).
        Some(tx) => {
            let _ = tx.send(body);
        }
        None => match body {
            Some((content_type, bytes)) => gosub_shared::subresource::complete(url.as_str(), content_type, bytes),
            None => gosub_shared::subresource::abandon(url.as_str()),
        },
    }
}

#[async_trait]
pub trait HtmlPipeline<C: RenderConfiguration> {
    async fn parse_stream(
        &mut self,
        request: FetchRequest,
        handle: FetchHandle,
        meta: FetchResultMeta,
        peek_buf: PeekBuf,
        body: Arc<SharedBody>,
    ) -> anyhow::Result<EngineDocument<C>>;

    async fn parse_bytes(
        &mut self,
        request: FetchRequest,
        handle: FetchHandle,
        meta: FetchResultMeta,
        body: &[u8],
    ) -> anyhow::Result<EngineDocument<C>>;
}

pub struct HtmlPipelineImpl<C: RenderConfiguration> {
    io_tx: IoChannel,
    zone_id: ZoneId,
    /// `Accept-Language` header value sent with discovered subresource requests.
    accept_language: Option<String>,
    /// Max document size in bytes (`net.document.max_bytes`); larger documents are truncated.
    max_document_bytes: usize,
    /// Where `@font-face` fonts are registered, once fetched.
    font_system: Arc<Mutex<C::FontSystem>>,
}

impl<C: RenderConfiguration> HtmlPipelineImpl<C> {
    pub fn new(
        zone_id: ZoneId,
        io_tx: IoChannel,
        accept_language: Option<String>,
        max_document_bytes: usize,
        font_system: Arc<Mutex<C::FontSystem>>,
    ) -> Self {
        Self {
            io_tx,
            zone_id,
            accept_language,
            max_document_bytes,
            font_system,
        }
    }

    async fn parse_with_reader<R>(
        &mut self,
        request: FetchRequest,
        handle: FetchHandle,
        meta: FetchResultMeta,
        reader: R,
    ) -> anyhow::Result<EngineDocument<C>>
    where
        R: AsyncRead + Unpin + Send + 'static,
    {
        // The main document's request is referenced by the navigation that started it, so
        // its timings can be attributed without threading a scope through the fetch stack.
        // Sub-resources reference a Document instead, which carries no navigation - those
        // stay unattributed until that mapping exists.
        let timing_scope = crate::net::req_ref_tracker::REF_REGISTRY
            .from_net(request.reference)
            .and_then(|r| match r {
                crate::net::req_ref_tracker::RequestReference::Navigation(nav_id) => {
                    Some(gosub_shared::timing::ScopeId(nav_id.0))
                }
                _ => None,
            });

        // Filled in below, once the pieces the gate needs exist. The parse only reaches for
        // it when a blocking script forces the issue.
        let mut cfg = crate::html::HtmlParseConfig {
            max_bytes: self.max_document_bytes,
            stylesheets: None,
            timing_scope,
        };

        let io_tx = self.io_tx.clone();
        let zone_id = self.zone_id;
        let parent_ref = request.reference;
        let parent_cancel = handle.cancel.clone();

        let child_handles = Arc::new(Mutex::new(Vec::<FetchHandle>::new()));
        let child_tasks = Arc::new(Mutex::new(Vec::<JoinHandle<()>>::new()));

        // Stylesheet bodies, delivered straight to the code below rather than through the
        // global hand-off. The parser no longer fetches them, so this parse is the only
        // consumer -- a per-parse channel says that, where a process-wide store keyed by URL
        // would leave it looking like anyone's to take.
        let sheet_bodies: SheetBodies = Arc::new(Mutex::new(HashMap::new()));

        let child_handles_for_closure = child_handles.clone();
        let child_tasks_for_closure = child_tasks.clone();
        let sheet_bodies_for_closure = sheet_bodies.clone();

        let mut sub_headers = http::HeaderMap::new();
        if let Some(langs) = &self.accept_language {
            if let Ok(val) = langs.parse() {
                sub_headers.insert(http::header::ACCEPT_LANGUAGE, val);
            }
        }

        let doc_url = meta.final_url.clone();
        let mut on_discover = |hint: ResourceHint| {
            // A remote document must never pull file:// subresources; don't even submit
            // them (the file loader refuses them again as defense in depth).
            if hint.url.scheme() == "file" && doc_url.scheme() != "file" {
                log::warn!(
                    "refusing file:// subresource {} for remote document {}",
                    hint.url,
                    doc_url
                );
                return;
            }
            let sub_req_id = RequestId::new();
            REF_REGISTRY.register_request(sub_req_id, hint.kind, Initiator::Parser);
            let mut headers = sub_headers.clone();
            if let Ok(val) = hint.kind.accept_header().parse() {
                headers.insert(http::header::ACCEPT, val);
            }
            // The referrer serves double duty: gosub-sonar computes the Referer header from
            // it (never for non-http(s) referrers), and the file loader uses it to accept
            // subresources of file:// documents.
            let sub_url = hint.url.clone();
            let sub_req = FetchRequest::builder(Method::GET, hint.url)
                .with_req_id(sub_req_id)
                .with_reference(parent_ref)
                .with_priority(hint.priority)
                .with_initiator(Initiator::Parser.to_net())
                .with_kind(hint.kind.to_net())
                .with_headers(headers)
                .with_referrer(doc_url.clone())
                // Buffered rather than streamed: the body is the point now. It is handed to
                // whichever consumer needs it -- the CSS parser, the media store, the font
                // loader -- each of which used to fetch the same URL a second time over its
                // own blocking client because these bytes were dropped on the floor.
                .with_streaming(false)
                .with_auto_decode(true)
                .build();

            // A stylesheet goes to this parse's own channel; everything else is announced in
            // the global hand-off, where the media store and the font loader look for it.
            let is_stylesheet = matches!(hint.kind, crate::net::types::ResourceKind::Stylesheet);
            let sheet_tx = if is_stylesheet {
                let (tx, rx) = tokio::sync::oneshot::channel();
                sheet_bodies_for_closure.lock().insert(sub_url.to_string(), rx);
                Some(tx)
            } else {
                gosub_shared::subresource::begin(sub_url.as_str());
                None
            };

            let io_tx_cloned = io_tx.clone();
            let parent_cancel_cloned = parent_cancel.clone();
            let child_handles = child_handles_for_closure.clone();
            let child_tasks = child_tasks_for_closure.clone();

            // Parent cancelled, so we don't have to do anything
            if parent_cancel_cloned.is_cancelled() {
                return;
            }

            let join_handle = spawn_named("html-sub-resource", async move {
                match submit_to_io(zone_id, sub_req, io_tx_cloned, Some(parent_cancel_cloned)).await {
                    Ok((child_handle, rx)) => {
                        child_handles.lock().push(child_handle);

                        let delivered = match rx.await {
                            Ok(FetchResult::Buffered { meta, body }) if meta.status == 200 && !body.is_empty() => {
                                Some((meta.content_type.clone(), body.to_vec()))
                            }
                            // Anything else -- a non-200, an empty body, a stream we did not
                            // ask for, a cancellation -- leaves nothing to hand on. Say so
                            // rather than staying silent, or a consumer waits out the timeout
                            // for bytes that are never coming.
                            _ => None,
                        };
                        deliver(sheet_tx, &sub_url, delivered);
                    }
                    Err(e) => {
                        log::warn!("Failed to submit discovered resource request: {:?}", e);
                        deliver(sheet_tx, &sub_url, None);
                    }
                }
            });

            child_tasks.lock().push(join_handle);
        };

        cfg.stylesheets = Some(Arc::new(ParseSheetGate {
            bodies: sheet_bodies.clone(),
            runtime: tokio::runtime::Handle::current(),
            zone_id,
            io_tx: io_tx.clone(),
            parent_ref,
            parent_cancel: parent_cancel.clone(),
            headers: sub_headers.clone(),
            referrer: doc_url.clone(),
        }));

        let was_cancelled = handle.cancel.is_cancelled();

        let _doc_timer = timing_guard!(gosub_shared::timing::Timing::HtmlDocument, meta.final_url.as_str());
        let res = parse_main_document_stream(
            meta.final_url, // This is the base URL
            reader,
            handle.cancel.clone(),
            cfg,
            &mut on_discover,
        )
        .await;

        // Put the linked stylesheets in place. This runs *before* the cancellation below,
        // which would otherwise take the fetches down with it -- and before the document is
        // handed on, so what the tab receives is complete, exactly as it was when the parser
        // fetched the sheets itself.
        let res = match res {
            Ok(mut doc) => {
                let sheets = SubFetch {
                    zone_id,
                    io_tx: &io_tx,
                    parent_ref,
                    parent_cancel: &parent_cancel,
                    headers: &sub_headers,
                    referrer: &doc_url,
                };
                resolve_pending_stylesheets::<C>(&mut doc, &sheet_bodies, &sheets).await;
                // Fonts are declared in CSS, so they can only be known once the sheets are.
                // Registered before the document is handed on, which is what keeps the first
                // layout from measuring text in a fallback face and having to do it again.
                super::webfonts::load_web_fonts::<C>(&doc, &doc_url, &self.font_system, &sheets, timing_scope).await;
                Ok(doc)
            }
            Err(e) => Err(e),
        };

        // A navigation that failed or was abandoned takes its subresource fetches with it:
        // nothing is going to consume them. Cancelling the parent cancels every child token,
        // including ones the spawned submission tasks have not created yet.
        //
        // A navigation that *succeeded* does not. Those fetches are the images the document
        // is about to be laid out with, and cancelling them here meant almost every image on
        // a page was reported cancelled, only for the media store to fetch it all over again
        // through its own blocking client -- a second connection, a second transfer, and
        // neither of them visible in the network panel. They finish on their own now, or the
        // next navigation cancels them.
        if was_cancelled || res.is_err() {
            parent_cancel.cancel();

            let joins: Vec<JoinHandle<()>> = {
                let mut g = child_tasks.lock();
                std::mem::take(&mut *g)
            };

            for jh in joins {
                let _ = jh.await;
            }
        }

        res.map_err(|e| anyhow!("Failed to parse HTML document: {:?}", e))
    }
}

#[async_trait]
impl<C: RenderConfiguration> HtmlPipeline<C> for HtmlPipelineImpl<C> {
    async fn parse_stream(
        &mut self,
        request: FetchRequest,
        handle: FetchHandle,
        meta: FetchResultMeta,
        peek_buf: PeekBuf,
        shared: Arc<SharedBody>,
    ) -> anyhow::Result<EngineDocument<C>> {
        let reader = SharedBody::combined_reader(peek_buf, shared);
        self.parse_with_reader(request, handle, meta, reader).await
    }

    async fn parse_bytes(
        &mut self,
        request: FetchRequest,
        handle: FetchHandle,
        meta: FetchResultMeta,
        body: &[u8],
    ) -> anyhow::Result<EngineDocument<C>> {
        // parsing bytes is just creating a stream of those bytes and passing it to the stream reader
        let stream = stream::iter(vec![Ok::<Bytes, std::io::Error>(Bytes::copy_from_slice(body))]);
        let reader = StreamReader::new(stream);
        self.parse_with_reader(request, handle, meta, reader).await
    }
}

/// Serves stylesheets to a parse that has stopped at a script and cannot go on without them.
///
/// Holds everything needed to answer from a thread that is not the runtime's: the receivers
/// for the fetches already in flight, and the means to start one for a link the document scan
/// did not see.
#[derive(Debug)]
struct ParseSheetGate {
    bodies: SheetBodies,
    runtime: tokio::runtime::Handle,
    zone_id: ZoneId,
    io_tx: IoChannel,
    parent_ref: gosub_sonar::RequestReference,
    parent_cancel: tokio_util::sync::CancellationToken,
    headers: http::HeaderMap,
    referrer: url::Url,
}

impl gosub_html5::parser::StylesheetSource for ParseSheetGate {
    fn fetch_blocking(&self, urls: &[String]) -> Vec<Option<Vec<u8>>> {
        // `block_on` from inside a `spawn_blocking` thread, which is allowed precisely
        // because it is not a runtime worker: this thread is meant to sit still. The same
        // call from a worker would deadlock the runtime it is waiting on.
        self.runtime.block_on(async {
            let mut out = Vec::with_capacity(urls.len());
            for url in urls {
                let waiting = self.bodies.lock().remove(url);
                let body = match waiting {
                    Some(rx) => rx.await.ok().flatten(),
                    None => {
                        fetch_subresource(
                            url,
                            crate::net::types::ResourceKind::Stylesheet,
                            &SubFetch {
                                zone_id: self.zone_id,
                                io_tx: &self.io_tx,
                                parent_ref: self.parent_ref,
                                parent_cancel: &self.parent_cancel,
                                headers: &self.headers,
                                referrer: &self.referrer,
                            },
                        )
                        .await
                    }
                };
                out.push(body.map(|(_, bytes)| bytes));
            }
            out
        })
    }
}

/// What the post-parse stages need to fetch something the document scan missed.
pub(crate) struct SubFetch<'a> {
    pub(crate) zone_id: ZoneId,
    pub(crate) io_tx: &'a IoChannel,
    pub(crate) parent_ref: gosub_sonar::RequestReference,
    pub(crate) parent_cancel: &'a tokio_util::sync::CancellationToken,
    pub(crate) headers: &'a http::HeaderMap,
    pub(crate) referrer: &'a url::Url,
}

/// Fetch and parse the stylesheets the parser recorded, and slot them into the cascade.
///
/// The parser records a `<link rel=stylesheet>` and moves on; this is where the sheet
/// actually arrives. Nearly all of them are already in flight -- the document scan submits
/// every link it can see before the parse even starts -- so this is usually a wait on a
/// fetch that is nearly done, not the start of one.
///
/// A sheet that fails to load leaves no gap and no error: the document renders without it,
/// which is what a browser does and what this code did when the parser fetched them itself.
async fn resolve_pending_stylesheets<C: RenderConfiguration>(
    doc: &mut EngineDocument<C>,
    bodies: &SheetBodies,
    fetch: &SubFetch<'_>,
) {
    use gosub_interface::css3::{CssOrigin, CssSystem};
    use gosub_interface::document::Document as _;

    let mut pending = doc.take_pending_stylesheets();
    if pending.is_empty() {
        return;
    }
    // Ascending, so the running offset below is correct.
    pending.sort_by_key(|(position, _)| *position);

    let mut inserted = 0usize;
    for (position, url) in pending {
        // Taken before the await: holding the guard across it would make this future
        // non-Send, and the whole pipeline with it.
        let waiting = bodies.lock().remove(&url);
        let body = match waiting {
            // Already in flight from the document scan: wait for the bytes.
            Some(rx) => rx.await.ok().flatten(),
            // The scan did not see this link -- it reads raw HTML with a regex, and a
            // `<link>` can be written in ways it does not match. Fetch it now, through the
            // same path, rather than reaching for a client of our own.
            None => fetch_subresource(&url, crate::net::types::ResourceKind::Stylesheet, fetch).await,
        };

        let Some((content_type, bytes)) = body else {
            log::warn!("Could not load external stylesheet from {url}");
            continue;
        };
        match content_type {
            Some(ref ct) if !ct.starts_with("text/css") => {
                log::warn!("External stylesheet has unexpected content type: {ct}");
            }
            None => log::warn!("External stylesheet has no content type: {url}"),
            _ => {}
        }
        let Ok(css) = String::from_utf8(bytes) else {
            log::warn!("External stylesheet from {url} is not valid UTF-8");
            continue;
        };

        let config = gosub_shared::config::ParserConfig {
            source: Some(url.clone()),
            ignore_errors: true,
            ..Default::default()
        };
        match <C::CssSystem as CssSystem>::parse_str(&css, config, CssOrigin::Author, &url) {
            Ok(sheet) => {
                // Everything already slotted in sat at or before this position, so each one
                // shifts this sheet one place further along.
                doc.insert_stylesheet(position + inserted, sheet);
                inserted += 1;
            }
            Err(err) => log::warn!("Error while parsing CSS stylesheet from {url}: {err}"),
        }
    }
}

/// Fetch one subresource through the zone's fetcher and wait for it.
pub(crate) async fn fetch_subresource(
    url: &str,
    kind: crate::net::types::ResourceKind,
    fetch: &SubFetch<'_>,
) -> SheetBody {
    let parsed = url::Url::parse(url).ok()?;
    // The same refusal the document scan applies: a page from the network does not get to
    // read the disk because it named the file somewhere the scanner could not see.
    if parsed.scheme() == "file" && fetch.referrer.scheme() != "file" {
        log::warn!(
            "refusing file:// subresource {parsed} for remote document {}",
            fetch.referrer
        );
        return None;
    }

    let req_id = RequestId::new();
    REF_REGISTRY.register_request(req_id, kind, Initiator::Parser);
    let req = FetchRequest::builder(Method::GET, parsed)
        .with_req_id(req_id)
        .with_reference(fetch.parent_ref)
        .with_priority(crate::net::types::Priority::High)
        .with_initiator(Initiator::Parser.to_net())
        .with_kind(kind.to_net())
        .with_headers(fetch.headers.clone())
        .with_referrer(fetch.referrer.clone())
        .with_streaming(false)
        .with_auto_decode(true)
        .build();

    let (_handle, rx) = submit_to_io(
        fetch.zone_id,
        req,
        fetch.io_tx.clone(),
        Some(fetch.parent_cancel.clone()),
    )
    .await
    .ok()?;
    match rx.await {
        Ok(FetchResult::Buffered { meta, body }) if meta.status == 200 && !body.is_empty() => {
            Some((meta.content_type.clone(), body.to_vec()))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::IoCommand;
    use crate::html::DefaultRenderConfig;
    use crate::net::req_ref_tracker::RequestReference;
    use crate::net::types::{Priority, ResourceKind};
    use crate::NavigationId;
    use std::time::Duration;
    use tokio::sync::mpsc;
    use tokio::time::sleep;
    use url::Url;

    // Minimal HTML that triggers 3 resource discoveries: link/script/img + a title.
    const HTML_WITH_RESOURCES: &str = r#"
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

    fn test_meta(base: &str) -> FetchResultMeta {
        let mut meta = FetchResultMeta::synthetic(Url::parse(base).expect("valid url"));
        meta.has_body = true;
        meta
    }

    fn test_request(base: &str) -> (FetchRequest, FetchHandle) {
        let req = FetchRequest::builder(Method::GET, Url::parse(base).unwrap())
            .with_req_id(RequestId::new())
            .with_reference(REF_REGISTRY.to_net(RequestReference::Navigation(NavigationId::new())))
            .with_priority(Priority::High)
            .with_kind(ResourceKind::Document.to_net())
            .with_initiator(Initiator::Parser.to_net())
            .with_streaming(true)
            .with_auto_decode(true)
            .build();

        let handle = FetchHandle {
            req_id: req.req_id,
            cancel: tokio_util::sync::CancellationToken::new(),
        };

        (req, handle)
    }

    /// Helper: start a dummy IO receiver that records child handles and immediately drops reply_tx.
    fn start_dummy_io() -> (IoChannel, Arc<Mutex<Vec<FetchHandle>>>) {
        let (tx, mut rx) = mpsc::unbounded_channel::<IoCommand>();
        let seen_children: Arc<Mutex<Vec<FetchHandle>>> = Arc::new(Mutex::new(vec![]));
        let seen_children_clone = seen_children.clone();

        tokio::spawn(async move {
            while let Some(cmd) = rx.recv().await {
                match cmd {
                    IoCommand::Fetch {
                        zone_id: _,
                        req: _,
                        handle,
                        reply_tx,
                    } => {
                        // record the child handle so tests can inspect cancellation state later
                        seen_children_clone.lock().push(handle);
                        // drop the sender to unblock the pipeline's `rx.await` without crafting a FetchResult
                        drop(reply_tx);
                    }
                    IoCommand::Decision { .. } => { /* not used here */ }
                    IoCommand::ShutdownZone { reply_tx, .. } => {
                        let _ = reply_tx.send(());
                    }
                }
            }
        });

        (tx, seen_children)
    }

    #[tokio::test(flavor = "current_thread")]
    async fn parse_bytes_discovers_and_submits_subresources() {
        // Arrange
        let (io_tx, seen_children) = start_dummy_io();
        let zone_id = ZoneId::new();
        let mut pipeline = HtmlPipelineImpl::<DefaultRenderConfig>::new(
            zone_id,
            io_tx,
            None,
            10 * 1024 * 1024,
            Arc::new(Mutex::new(Default::default())),
        );

        let (req, handle) = test_request("https://example.com/path/index.html");
        let meta = test_meta("https://example.com/path/index.html");
        let body = HTML_WITH_RESOURCES.as_bytes();

        // Act
        let doc = HtmlPipeline::<DefaultRenderConfig>::parse_bytes(&mut pipeline, req, handle, meta, body)
            .await
            .expect("parse_bytes should succeed");

        // Allow spawned tasks to submit to IO and be recorded
        sleep(Duration::from_millis(10)).await;

        // Assert: title extracted from DOM
        assert_eq!(crate::html::document_title(&doc).as_deref(), Some("Hello World"));

        // Assert: 3 subresources were submitted (stylesheet, script, image)
        let count = seen_children.lock().len();
        assert_eq!(count, 3, "expected 3 subresource fetches, saw {}", count);
    }

    /// The rule this replaces was "cancel every subresource when the parse ends", which read
    /// as tidiness and behaved as waste: an image still in flight was cancelled and then
    /// fetched all over again by the media store, on a second connection, out of sight of
    /// the network panel. On a page whose images are slower than its HTML -- which is most
    /// pages -- that was nearly all of them.
    #[tokio::test(flavor = "current_thread")]
    async fn parse_bytes_leaves_subresource_fetches_running_when_the_parse_succeeds() {
        // Arrange
        let (io_tx, seen_children) = start_dummy_io();
        let zone_id = ZoneId::new();
        let mut pipeline = HtmlPipelineImpl::<DefaultRenderConfig>::new(
            zone_id,
            io_tx,
            None,
            10 * 1024 * 1024,
            Arc::new(Mutex::new(Default::default())),
        );

        let (req, handle) = test_request("https://example.com/");
        let meta = test_meta("https://example.com/");
        let body = HTML_WITH_RESOURCES.as_bytes();

        // Act
        let _ = HtmlPipeline::<DefaultRenderConfig>::parse_bytes(&mut pipeline, req, handle, meta, body)
            .await
            .expect("parse ok");

        // Give the pipeline a tick to do anything it means to do after the parse.
        sleep(Duration::from_millis(10)).await;

        // Assert: the fetches are still alive, because the document is about to be laid out
        // with what they bring back.
        let children = seen_children.lock();
        assert!(!children.is_empty(), "expected subresource children to be recorded");
        for h in children.iter() {
            assert!(
                !h.cancel.is_cancelled(),
                "a subresource fetch should outlive a parse that succeeded"
            );
        }
    }

    /// The other half of the rule: a navigation nobody is waiting for takes its fetches with
    /// it. Cancelled before the parse begins, so nothing is submitted at all.
    #[tokio::test(flavor = "current_thread")]
    async fn parse_bytes_submits_nothing_for_an_abandoned_navigation() {
        let (io_tx, seen_children) = start_dummy_io();
        let zone_id = ZoneId::new();
        let mut pipeline = HtmlPipelineImpl::<DefaultRenderConfig>::new(
            zone_id,
            io_tx,
            None,
            10 * 1024 * 1024,
            Arc::new(Mutex::new(Default::default())),
        );

        let (req, handle) = test_request("https://example.com/");
        handle.cancel.cancel();
        let meta = test_meta("https://example.com/");

        let result = HtmlPipeline::<DefaultRenderConfig>::parse_bytes(
            &mut pipeline,
            req,
            handle,
            meta,
            HTML_WITH_RESOURCES.as_bytes(),
        )
        .await;

        assert!(result.is_err(), "an abandoned navigation should not produce a document");
        sleep(Duration::from_millis(10)).await;
        assert!(
            seen_children.lock().is_empty(),
            "an abandoned navigation should not fetch subresources"
        );
    }
}
