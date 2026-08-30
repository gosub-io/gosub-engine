use crate::engine::types::{IoChannel, PeekBuf, RequestId};
use crate::html::{parse_main_document_stream, EngineDocument, RenderConfiguration, ResourceHint};
use crate::net::brokered_loader::BrokeredLoader;
use crate::net::req_ref_tracker::REF_REGISTRY;
use crate::net::types::{FetchHandle, FetchRequest, FetchResultMeta, Initiator};
use crate::net::{submit_to_io, SharedBody};
use crate::tab::TabId;
use crate::util::spawn_named;
use crate::zone::ZoneId;
use anyhow::anyhow;
use async_trait::async_trait;
use bytes::Bytes;
use futures_util::stream;
use gosub_shared::timing_guard;
use http::Method;
use parking_lot::Mutex;
use std::sync::Arc;
use tokio::io::AsyncRead;
use tokio::task::JoinHandle;
use tokio_util::io::StreamReader;

/// What the pipeline made of a document body: the parsed document, with its
/// source when a renderer process may re-parse it.
pub struct ParsedDocument<C: RenderConfiguration> {
    pub doc: Box<EngineDocument<C>>,
    pub source: Option<Arc<str>>,
}

impl<C: RenderConfiguration> ParsedDocument<C> {
    pub fn into_parts(self) -> (EngineDocument<C>, Option<Arc<str>>) {
        (*self.doc, self.source)
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
    ) -> anyhow::Result<ParsedDocument<C>>;

    async fn parse_bytes(
        &mut self,
        request: FetchRequest,
        handle: FetchHandle,
        meta: FetchResultMeta,
        body: &[u8],
    ) -> anyhow::Result<ParsedDocument<C>>;
}

pub struct HtmlPipelineImpl {
    io_tx: IoChannel,
    zone_id: ZoneId,
    /// The tab these subresources belong to, so the I/O side can attach its
    /// cookies. Subresources previously carried none at all.
    tab_id: TabId,
    /// `Accept-Language` header value sent with discovered subresource requests.
    accept_language: Option<String>,
    /// Max document size in bytes (`net.document.max_bytes`); larger documents are truncated.
    max_document_bytes: usize,
    /// Also return the parsed document's source text (see
    /// `HtmlParseConfig::capture_source`) - on when the engine renders
    /// out-of-process and its renderer will need to re-parse.
    capture_source: bool,
}

impl HtmlPipelineImpl {
    pub fn new(
        zone_id: ZoneId,
        tab_id: TabId,
        io_tx: IoChannel,
        accept_language: Option<String>,
        max_document_bytes: usize,
        capture_source: bool,
    ) -> Self {
        Self {
            io_tx,
            zone_id,
            tab_id,
            accept_language,
            max_document_bytes,
            capture_source,
        }
    }

    async fn parse_with_reader<C, R>(
        &mut self,
        request: FetchRequest,
        handle: FetchHandle,
        meta: FetchResultMeta,
        reader: R,
    ) -> anyhow::Result<ParsedDocument<C>>
    where
        C: RenderConfiguration,
        R: AsyncRead + Unpin + Send + 'static,
    {
        let io_tx = self.io_tx.clone();
        let zone_id = self.zone_id;
        let tab_id = self.tab_id;
        let parent_ref = request.reference;
        let parent_cancel = handle.cancel.clone();

        let cfg = crate::html::HtmlParseConfig {
            max_bytes: self.max_document_bytes,
            capture_source: self.capture_source,
            // The parse happens on this tab's behalf, so its stylesheet loads carry
            // the tab's identity and cookies like any other request — and are
            // cancelled with the parse that wanted them.
            resource_loader: Some(
                BrokeredLoader::new(zone_id, Some(tab_id), io_tx.clone())
                    .with_cancel(&parent_cancel)
                    .shared(),
            ),
        };

        let child_handles = Arc::new(Mutex::new(Vec::<FetchHandle>::new()));
        let child_tasks = Arc::new(Mutex::new(Vec::<JoinHandle<()>>::new()));

        let child_handles_for_closure = child_handles.clone();
        let child_tasks_for_closure = child_tasks.clone();

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
            let sub_req = FetchRequest::builder(Method::GET, hint.url)
                .with_req_id(sub_req_id)
                .with_reference(parent_ref)
                .with_priority(hint.priority)
                .with_initiator(Initiator::Parser.to_net())
                .with_kind(hint.kind.to_net())
                .with_headers(headers)
                .with_referrer(doc_url.clone())
                .with_streaming(true)
                .with_auto_decode(true)
                .build();

            let io_tx_cloned = io_tx.clone();
            let parent_cancel_cloned = parent_cancel.clone();
            let child_handles = child_handles_for_closure.clone();
            let child_tasks = child_tasks_for_closure.clone();

            // Parent cancelled, so we don't have to do anything
            if parent_cancel_cloned.is_cancelled() {
                return;
            }

            let join_handle = spawn_named("html-sub-resource", async move {
                match submit_to_io(zone_id, Some(tab_id), sub_req, io_tx_cloned, Some(parent_cancel_cloned)).await {
                    Ok((child_handle, rx)) => {
                        child_handles.lock().push(child_handle);

                        let _ = rx.await;
                    }
                    Err(e) => {
                        log::warn!("Failed to submit discovered resource request: {:?}", e);
                    }
                }
            });

            child_tasks.lock().push(join_handle);
        };

        let was_cancelled = handle.cancel.is_cancelled();

        let _doc_timer = timing_guard!("html.document", meta.final_url.as_str());
        let res = parse_main_document_stream(
            meta.final_url, // This is the base URL
            reader,
            handle.cancel.clone(),
            cfg,
            &mut on_discover,
        )
        .await;

        // Cancel the parent token so that all child fetch tokens (which are children of
        // parent_cancel via child_token()) are also cancelled. This works regardless of
        // whether the spawned submission tasks have run yet, since the cancellation
        // propagates to any child tokens created from parent_cancel in the future too.
        parent_cancel.cancel();

        // On error or parent cancellation, also await all child tasks to clean up.
        if was_cancelled || res.is_err() {
            let joins: Vec<JoinHandle<()>> = {
                let mut g = child_tasks.lock();
                std::mem::take(&mut *g)
            };

            for jh in joins {
                let _ = jh.await;
            }
        }

        res.map(|(doc, source)| ParsedDocument {
            doc: Box::new(doc),
            source,
        })
        .map_err(|e| anyhow!("Failed to parse HTML document: {:?}", e))
    }
}

#[async_trait]
impl<C: RenderConfiguration> HtmlPipeline<C> for HtmlPipelineImpl {
    async fn parse_stream(
        &mut self,
        request: FetchRequest,
        handle: FetchHandle,
        meta: FetchResultMeta,
        peek_buf: PeekBuf,
        shared: Arc<SharedBody>,
    ) -> anyhow::Result<ParsedDocument<C>> {
        let reader = SharedBody::combined_reader(peek_buf, shared);
        self.parse_with_reader::<C, _>(request, handle, meta, reader).await
    }

    async fn parse_bytes(
        &mut self,
        request: FetchRequest,
        handle: FetchHandle,
        meta: FetchResultMeta,
        body: &[u8],
    ) -> anyhow::Result<ParsedDocument<C>> {
        // parsing bytes is just creating a stream of those bytes and passing it to the stream reader
        let stream = stream::iter(vec![Ok::<Bytes, std::io::Error>(Bytes::copy_from_slice(body))]);
        let reader = StreamReader::new(stream);
        self.parse_with_reader::<C, _>(request, handle, meta, reader).await
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
        FetchResultMeta {
            final_url: Url::parse(base).expect("valid url"),
            status: 200,
            status_text: "OK".into(),
            headers: http::HeaderMap::new(),
            content_length: None,
            content_type: None,
            has_body: true,
            tainting: gosub_sonar::ResponseTainting::Basic,
        }
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
                        tab_id: _,
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

    // Multi-threaded on purpose: parsing blocks on the brokered stylesheet load,
    // so the task answering IoCommands needs a thread of its own. On a
    // current-thread runtime that load can only time out — the same constraint
    // `net::brokered_loader` warns embedders about.
    #[tokio::test(flavor = "multi_thread")]
    async fn parse_bytes_discovers_and_submits_subresources() {
        // Arrange
        let (io_tx, seen_children) = start_dummy_io();
        let zone_id = ZoneId::new();
        let mut pipeline = HtmlPipelineImpl::new(zone_id, TabId::new(), io_tx, None, 10 * 1024 * 1024, false);

        let (req, handle) = test_request("https://example.com/path/index.html");
        let meta = test_meta("https://example.com/path/index.html");
        let body = HTML_WITH_RESOURCES.as_bytes();

        // Act
        let (doc, _source) = HtmlPipeline::<DefaultRenderConfig>::parse_bytes(&mut pipeline, req, handle, meta, body)
            .await
            .expect("parse_bytes should succeed")
            .into_parts();

        // Allow spawned tasks to submit to IO and be recorded
        sleep(Duration::from_millis(10)).await;

        // Assert: title extracted from DOM
        assert_eq!(crate::html::document_title(&doc).as_deref(), Some("Hello World"));

        // Three warm-up fetches from regex discovery (stylesheet, script, image),
        // plus the parser's own brokered load of the `<link rel="stylesheet">` —
        // which used to bypass this channel entirely by going straight to the network.
        let count = seen_children.lock().len();
        assert_eq!(count, 4, "expected 4 fetches, saw {}", count);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn parse_bytes_cancels_children_on_finish() {
        // Arrange
        let (io_tx, seen_children) = start_dummy_io();
        let zone_id = ZoneId::new();
        let mut pipeline = HtmlPipelineImpl::new(zone_id, TabId::new(), io_tx, None, 10 * 1024 * 1024, false);

        let (req, handle) = test_request("https://example.com/");
        let meta = test_meta("https://example.com/");
        let body = HTML_WITH_RESOURCES.as_bytes();

        // Act
        let _ = HtmlPipeline::<DefaultRenderConfig>::parse_bytes(&mut pipeline, req, handle, meta, body)
            .await
            .expect("parse ok");

        // Give the pipeline a tick to run the post-parse cancellation
        sleep(Duration::from_millis(10)).await;

        // Assert: all recorded children are canceled (pipeline proactively cancels them at end)
        let children = seen_children.lock();
        assert!(!children.is_empty(), "expected subresource children to be recorded");
        for h in children.iter() {
            assert!(
                h.cancel.is_cancelled(),
                "child handle should be canceled after parse end"
            );
        }
    }
}
