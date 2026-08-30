use crate::engine::types::{IoChannel, PeekBuf};
use crate::html::{parse_main_document_stream, EngineDocument, RenderConfiguration};
use crate::net::brokered_loader::BrokeredLoader;
use crate::net::types::{FetchHandle, FetchRequest, FetchResultMeta};
use crate::net::SharedBody;
use crate::tab::TabId;
use crate::zone::ZoneId;
use anyhow::anyhow;
use async_trait::async_trait;
use bytes::Bytes;
use futures_util::stream;
use gosub_shared::timing_guard;
use std::sync::Arc;
use tokio::io::AsyncRead;
use tokio_util::io::StreamReader;

/// What the pipeline made of a document body.
pub enum ParsedDocument<C: RenderConfiguration> {
    /// Parsed here, with its source when a renderer process may re-parse it.
    Parsed {
        doc: Box<EngineDocument<C>>,
        source: Option<Arc<str>>,
    },
    /// Not parsed here: the renderer process parses. Only the source is kept.
    SourceOnly { source: Arc<str> },
}

impl<C: RenderConfiguration> ParsedDocument<C> {
    pub fn into_parts(self) -> (Option<EngineDocument<C>>, Option<Arc<str>>) {
        match self {
            Self::Parsed { doc, source } => (Some(*doc), source),
            Self::SourceOnly { source } => (None, Some(source)),
        }
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
    /// Max document size in bytes (`net.document.max_bytes`); larger documents are truncated.
    max_document_bytes: usize,
    /// Also return the parsed document's source text (see
    /// `HtmlParseConfig::capture_source`) - on when the engine renders
    /// out-of-process and its renderer will need to re-parse.
    capture_source: bool,
    /// Keep only the source: the renderer process parses, this process never
    /// runs the HTML parser on page content.
    source_only: bool,
}

impl HtmlPipelineImpl {
    /// Skip parsing here and keep the source for a renderer process.
    pub fn source_only(mut self, on: bool) -> Self {
        self.source_only = on;
        self
    }

    pub fn new(
        zone_id: ZoneId,
        tab_id: TabId,
        io_tx: IoChannel,
        max_document_bytes: usize,
        capture_source: bool,
    ) -> Self {
        Self {
            source_only: false,
            io_tx,
            zone_id,
            tab_id,
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
        if self.source_only {
            let source = crate::html::read_document_source(
                &meta.final_url,
                reader,
                handle.cancel.clone(),
                self.max_document_bytes,
            )
            .await
            .map_err(|e| anyhow!("Failed to read HTML document: {:?}", e))?;
            handle.cancel.cancel();
            return Ok(ParsedDocument::SourceOnly { source });
        }
        let _ = request;
        let parent_cancel = handle.cancel.clone();

        let cfg = crate::html::HtmlParseConfig {
            max_bytes: self.max_document_bytes,
            capture_source: self.capture_source,
            // The parse happens on this tab's behalf, so its stylesheet loads carry
            // the tab's identity and cookies like any other request - and are
            // cancelled with the parse that wanted them.
            resource_loader: Some(
                BrokeredLoader::new(self.zone_id, Some(self.tab_id), self.io_tx.clone())
                    .with_cancel(&parent_cancel)
                    .shared(),
            ),
        };

        let _doc_timer = timing_guard!("html.document", meta.final_url.as_str());
        let res = parse_main_document_stream(meta.final_url, reader, handle.cancel.clone(), cfg).await;

        // The parse is over: nothing it started may keep loading.
        parent_cancel.cancel();

        res.map(|(doc, source)| ParsedDocument::Parsed {
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
    use crate::engine::types::RequestId;
    use crate::events::IoCommand;
    use crate::html::DefaultRenderConfig;
    use crate::net::req_ref_tracker::{RequestReference, REF_REGISTRY};
    use crate::net::types::{Initiator, Priority, ResourceKind};
    use crate::NavigationId;
    use http::Method;
    use parking_lot::Mutex;
    use std::time::Duration;
    use tokio::sync::mpsc;
    use tokio::time::sleep;
    use url::Url;

    // A stylesheet the parser loads, plus a script and an image nothing here fetches.
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
    // current-thread runtime that load can only time out - the same constraint
    // `net::brokered_loader` warns embedders about.
    #[tokio::test(flavor = "multi_thread")]
    async fn parse_bytes_loads_the_stylesheet_once_and_nothing_else() {
        // Arrange
        let (io_tx, seen_children) = start_dummy_io();
        let zone_id = ZoneId::new();
        let mut pipeline = HtmlPipelineImpl::new(zone_id, TabId::new(), io_tx, 10 * 1024 * 1024, false);

        let (req, handle) = test_request("https://example.com/path/index.html");
        let meta = test_meta("https://example.com/path/index.html");
        let body = HTML_WITH_RESOURCES.as_bytes();

        // Act
        let (doc, _source) = HtmlPipeline::<DefaultRenderConfig>::parse_bytes(&mut pipeline, req, handle, meta, body)
            .await
            .expect("parse_bytes should succeed")
            .into_parts();
        let doc = doc.expect("parsed in-process");

        // Allow spawned tasks to submit to IO and be recorded
        sleep(Duration::from_millis(10)).await;

        // Assert: title extracted from DOM
        assert_eq!(crate::html::document_title(&doc).as_deref(), Some("Hello World"));

        // Exactly the parser's own brokered load of the `<link rel="stylesheet">`:
        // no prefetch of it (that was a second fetch of the same bytes), and no
        // fetch of the script or image, which nothing here consumes.
        let count = seen_children.lock().len();
        assert_eq!(count, 1, "expected 1 fetch (the stylesheet), saw {}", count);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn parse_bytes_cancels_children_on_finish() {
        // Arrange
        let (io_tx, seen_children) = start_dummy_io();
        let zone_id = ZoneId::new();
        let mut pipeline = HtmlPipelineImpl::new(zone_id, TabId::new(), io_tx, 10 * 1024 * 1024, false);

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
        assert!(!children.is_empty(), "expected the stylesheet load to be recorded");
        for h in children.iter() {
            assert!(
                h.cancel.is_cancelled(),
                "child handle should be canceled after parse end"
            );
        }
    }
}
