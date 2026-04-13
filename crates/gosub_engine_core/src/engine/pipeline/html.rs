use crate::engine::types::{IoChannel, PeekBuf, RequestId};
use crate::html::{hint_to_net, parse_html_stream, Html5ParseConfig, ResourceHint};
use crate::net::types::{FetchHandle, FetchKeyData, FetchRequest, FetchResultMeta, Initiator};
use crate::net::{submit_to_io, SharedBody};
use crate::zone::ZoneId;
use anyhow::anyhow;
use async_trait::async_trait;
use bytes::Bytes;
use futures_util::stream;
use gosub_interface::config::HasDocument;
use http::Method;
use std::marker::PhantomData;
use std::sync::{Arc, Mutex};
use tokio::io::AsyncRead;
use tokio::task::JoinHandle;
use tokio_util::io::StreamReader;

#[async_trait]
pub trait HtmlPipeline<C: HasDocument> {
    async fn parse_stream(
        &mut self,
        request: FetchRequest,
        handle: FetchHandle,
        meta: FetchResultMeta,
        peek_buf: PeekBuf,
        body: Arc<SharedBody>,
    ) -> anyhow::Result<C::Document>;

    async fn parse_bytes(
        &mut self,
        request: FetchRequest,
        handle: FetchHandle,
        meta: FetchResultMeta,
        body: &[u8],
    ) -> anyhow::Result<C::Document>;
}

pub struct HtmlPipelineImpl<C: HasDocument> {
    io_tx: IoChannel,
    zone_id: ZoneId,
    _phantom: PhantomData<C>,
}

impl<C: HasDocument> HtmlPipelineImpl<C> {
    pub fn new(zone_id: ZoneId, io_tx: IoChannel) -> Self {
        Self {
            io_tx,
            zone_id,
            _phantom: PhantomData,
        }
    }

    async fn parse_with_reader<R>(
        &mut self,
        request: FetchRequest,
        handle: FetchHandle,
        meta: FetchResultMeta,
        reader: R,
    ) -> anyhow::Result<C::Document>
    where
        R: AsyncRead + Unpin + Send + 'static,
    {
        let cfg = Html5ParseConfig::default();

        let io_tx = self.io_tx.clone();
        let zone_id: gosub_net::types::ZoneId = self.zone_id;
        let parent_ref = request.reference;
        let parent_cancel = handle.cancel.clone();

        let child_handles = Arc::new(Mutex::new(Vec::<FetchHandle>::new()));
        let child_tasks = Arc::new(Mutex::new(Vec::<JoinHandle<()>>::new()));

        let child_handles_for_closure = child_handles.clone();
        let child_tasks_for_closure = child_tasks.clone();

        let mut on_discover = |hint: ResourceHint| {
            let (url, kind, _dest, priority) = hint_to_net(hint);

            // Create a request for the discovered resource
            let sub_req = FetchRequest {
                req_id: RequestId::new(),
                reference: parent_ref,
                key_data: FetchKeyData {
                    url,
                    method: Method::GET,
                    headers: Default::default(),
                },
                priority,
                initiator: Initiator::Parser,
                kind,
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
                        child_handles.lock().unwrap().push(child_handle);

                        let _ = rx.await;
                    }
                    Err(e) => {
                        log::warn!("Failed to submit discovered resource request: {:?}", e);
                    }
                }
            });

            child_tasks.lock().unwrap().push(join_handle);
        };

        let was_cancelled = handle.cancel.is_cancelled();

        let res = parse_html_stream::<C, _, _>(
            meta.final_url, // This is the base URL
            reader,
            handle.cancel.clone(),
            cfg,
            &mut on_discover,
        )
        .await;

        if was_cancelled || res.is_err() {
            for h in child_handles.lock().unwrap().drain(..) {
                h.cancel.cancel();
            }

            let joins: Vec<JoinHandle<()>> = {
                let mut g = child_tasks.lock().unwrap();
                std::mem::take(&mut *g) // drain without keeping the guard alive
            };

            for jh in joins {
                let _ = jh.await;
            }
        }

        res.map_err(|e| anyhow!("Failed to parse HTML document: {:?}", e))
    }
}

#[async_trait]
impl<C: HasDocument + Send + Sync + 'static> HtmlPipeline<C> for HtmlPipelineImpl<C>
where
    C::Document: Send + Sync,
{
    async fn parse_stream(
        &mut self,
        request: FetchRequest,
        handle: FetchHandle,
        meta: FetchResultMeta,
        peek_buf: PeekBuf,
        shared: Arc<SharedBody>,
    ) -> anyhow::Result<C::Document> {
        let reader = SharedBody::combined_reader(peek_buf, shared);
        self.parse_with_reader(request, handle, meta, reader).await
    }

    async fn parse_bytes(
        &mut self,
        request: FetchRequest,
        handle: FetchHandle,
        meta: FetchResultMeta,
        body: &[u8],
    ) -> anyhow::Result<C::Document> {
        // parsing bytes is just creating a stream of those bytes and passing it to the stream reader
        let stream = stream::iter(vec![Ok::<Bytes, std::io::Error>(Bytes::copy_from_slice(body))]);
        let reader = StreamReader::new(stream);
        self.parse_with_reader(request, handle, meta, reader).await
    }
}
