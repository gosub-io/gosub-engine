use crate::engine::types::{IoChannel, PeekBuf, RequestId};
use crate::html::{parse_main_document_stream, DummyDocument, ResourceHint};
use crate::net::types::{FetchHandle, FetchKeyData, FetchRequest, FetchResultMeta, Initiator};
use crate::net::{submit_to_io, SharedBody};
use crate::zone::ZoneId;
use anyhow::anyhow;
use async_trait::async_trait;
use bytes::Bytes;
use futures_util::stream;
use http::Method;
use parking_lot::Mutex;
use std::sync::Arc;
use tokio::io::AsyncRead;
use tokio::task::JoinHandle;
use tokio_util::io::StreamReader;

#[async_trait]
pub trait HtmlPipeline {
    async fn parse_stream(
        &mut self,
        request: FetchRequest,
        handle: FetchHandle,
        meta: FetchResultMeta,
        peek_buf: PeekBuf,
        body: Arc<SharedBody>,
    ) -> anyhow::Result<DummyDocument>;

    async fn parse_bytes(
        &mut self,
        request: FetchRequest,
        handle: FetchHandle,
        meta: FetchResultMeta,
        body: &[u8],
    ) -> anyhow::Result<DummyDocument>;
}

pub struct HtmlPipelineImpl {
    io_tx: IoChannel,
    zone_id: ZoneId,
}

impl HtmlPipelineImpl {
    pub fn new(zone_id: ZoneId, io_tx: IoChannel) -> Self {
        Self { io_tx, zone_id }
    }

    async fn parse_with_reader<R>(
        &mut self,
        request: FetchRequest,
        handle: FetchHandle,
        meta: FetchResultMeta,
        reader: R,
    ) -> anyhow::Result<DummyDocument>
    where
        R: AsyncRead + Unpin + Send + 'static,
    {
        let cfg = crate::html::DummyHtml5Config::default();

        let io_tx = self.io_tx.clone();
        let zone_id = self.zone_id;
        let parent_ref = request.reference;
        let parent_cancel = handle.cancel.clone();

        let child_handles = Arc::new(Mutex::new(Vec::<FetchHandle>::new()));
        let child_tasks = Arc::new(Mutex::new(Vec::<JoinHandle<()>>::new()));

        let child_handles_for_closure = child_handles.clone();
        let child_tasks_for_closure = child_tasks.clone();

        let mut on_discover = |hint: ResourceHint| {
            // Create a request for the discovered resource
            let sub_req = FetchRequest {
                req_id: RequestId::new(),
                reference: parent_ref,
                key_data: FetchKeyData {
                    url: hint.url,
                    method: Method::GET,
                    headers: Default::default(),
                },
                priority: hint.priority,
                initiator: Initiator::Parser,
                kind: hint.kind,
                streaming: true,
                auto_decode: true,
                max_bytes: None,
            };

            let io_tx_cloned = io_tx.clone();
            let parent_cancel_cloned = parent_cancel.clone();
            let child_handles = child_handles_for_closure.clone();
            let child_tasks = child_tasks_for_closure.clone();

            // Parent cancelled, so we don't have to do anything
            if parent_cancel_cloned.is_cancelled() {
                return;
            }

            let join_handle = tokio::spawn(async move {
                match submit_to_io(zone_id, sub_req, io_tx_cloned, Some(parent_cancel_cloned)).await {
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

        res.map_err(|e| anyhow!("Failed to parse HTML document: {:?}", e))
    }
}

#[async_trait]
impl HtmlPipeline for HtmlPipelineImpl {
    async fn parse_stream(
        &mut self,
        request: FetchRequest,
        handle: FetchHandle,
        meta: FetchResultMeta,
        peek_buf: PeekBuf,
        shared: Arc<SharedBody>,
    ) -> anyhow::Result<DummyDocument> {
        let reader = SharedBody::combined_reader(peek_buf, shared);
        self.parse_with_reader(request, handle, meta, reader).await
    }

    async fn parse_bytes(
        &mut self,
        request: FetchRequest,
        handle: FetchHandle,
        meta: FetchResultMeta,
        body: &[u8],
    ) -> anyhow::Result<DummyDocument> {
        // parsing bytes is just creating a stream of those bytes and passing it to the stream reader
        let stream = stream::iter(vec![Ok::<Bytes, std::io::Error>(Bytes::copy_from_slice(body))]);
        let reader = StreamReader::new(stream);
        self.parse_with_reader(request, handle, meta, reader).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::IoCommand;
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
        let final_url = Url::parse(base).expect("valid url");
        // If your FetchResultMeta isn't Default, replace with the proper constructor.
        FetchResultMeta {
            final_url,
            ..Default::default()
        }
    }

    fn test_request(base: &str) -> (FetchRequest, FetchHandle) {
        let req = FetchRequest {
            req_id: RequestId::new(),
            reference: RequestReference::Navigation(NavigationId::new()), // or whatever your reference type needs
            key_data: FetchKeyData {
                url: Url::parse(base).unwrap(),
                method: Method::GET,
                headers: Default::default(),
            },
            priority: Priority::High,
            kind: ResourceKind::Document,
            initiator: Initiator::Parser,
            streaming: true,
            auto_decode: true,
            max_bytes: None,
        };

        let handle = FetchHandle {
            req_id: req.req_id,
            key: req.key_data.clone(),
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
        let mut pipeline = HtmlPipelineImpl::new(zone_id, io_tx);

        let (req, handle) = test_request("https://example.com/path/index.html");
        let meta = test_meta("https://example.com/path/index.html");
        let body = HTML_WITH_RESOURCES.as_bytes();

        // Act
        let doc = pipeline
            .parse_bytes(req, handle, meta, body)
            .await
            .expect("parse_bytes should succeed");

        // Allow spawned tasks to submit to IO and be recorded
        sleep(Duration::from_millis(10)).await;

        // Assert: title extracted
        assert_eq!(doc.title.as_deref(), Some("Hello World"));

        // Assert: 3 subresources were submitted (stylesheet, script, image)
        let count = seen_children.lock().len();
        assert_eq!(count, 3, "expected 3 subresource fetches, saw {}", count);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn parse_bytes_cancels_children_on_finish() {
        // Arrange
        let (io_tx, seen_children) = start_dummy_io();
        let zone_id = ZoneId::new();
        let mut pipeline = HtmlPipelineImpl::new(zone_id, io_tx);

        let (req, handle) = test_request("https://example.com/");
        let meta = test_meta("https://example.com/");
        let body = HTML_WITH_RESOURCES.as_bytes();

        // Act
        let _ = pipeline.parse_bytes(req, handle, meta, body).await.expect("parse ok");

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
