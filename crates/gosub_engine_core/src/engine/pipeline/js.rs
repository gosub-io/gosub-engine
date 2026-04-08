use crate::engine::types::PeekBuf;
use crate::net::types::FetchResultMeta;
use crate::net::{stream_to_bytes, SharedBody};
use async_trait::async_trait;
use bytes::Bytes;
use std::sync::Arc;

/// A script loaded from the network, ready to be handed to the JS engine.
///
/// Execution happens separately at the tab/page lifecycle level — this type
/// only represents the delivered bytes, not the result of running the script.
#[derive(Clone, Debug)]
pub struct LoadedScript {
    /// Raw script source bytes (UTF-8).
    pub source: Bytes,
    /// Final URL the script was fetched from.
    pub source_url: String,
}

#[async_trait]
pub trait JsPipeline: Send {
    async fn parse_stream(
        &mut self,
        meta: FetchResultMeta,
        peek_buf: PeekBuf,
        body: Arc<SharedBody>,
    ) -> anyhow::Result<LoadedScript>;

    async fn parse_bytes(&mut self, meta: FetchResultMeta, body: &[u8]) -> anyhow::Result<LoadedScript>;
}

pub struct JsPipelineImpl;

#[async_trait]
impl JsPipeline for JsPipelineImpl {
    async fn parse_stream(
        &mut self,
        meta: FetchResultMeta,
        peek_buf: PeekBuf,
        shared: Arc<SharedBody>,
    ) -> anyhow::Result<LoadedScript> {
        let buf = stream_to_bytes(peek_buf, shared).await?;
        self.parse_bytes(meta, buf.as_ref()).await
    }

    async fn parse_bytes(&mut self, meta: FetchResultMeta, body: &[u8]) -> anyhow::Result<LoadedScript> {
        Ok(LoadedScript {
            source: Bytes::copy_from_slice(body),
            source_url: meta.final_url.to_string(),
        })
    }
}
