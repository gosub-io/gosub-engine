use crate::engine::types::PeekBuf;
use crate::net::types::FetchResultMeta;
use crate::net::{stream_to_bytes, SharedBody};
use async_trait::async_trait;
use std::sync::Arc;

pub type DummyJsDocument = String;

#[async_trait]
pub trait JsPipeline {
    async fn parse_stream(
        &mut self,
        meta: FetchResultMeta,
        peek_buf: PeekBuf,
        body: Arc<SharedBody>,
    ) -> anyhow::Result<DummyJsDocument>;

    async fn parse_bytes(&mut self, meta: FetchResultMeta, body: &[u8]) -> anyhow::Result<DummyJsDocument>;
}

pub struct JsPipelineImpl;

#[async_trait]
impl JsPipeline for JsPipelineImpl {
    async fn parse_stream(
        &mut self,
        _meta: FetchResultMeta,
        peek_buf: PeekBuf,
        shared: Arc<SharedBody>,
    ) -> anyhow::Result<DummyJsDocument> {
        // Normally, we send chunks to the Font parser. Right now, we just collect everything
        match stream_to_bytes(peek_buf, shared).await {
            Ok(buf) => Ok(String::from_utf8_lossy(buf.as_ref()).to_string()),
            Err(e) => Err(anyhow::anyhow!("Failed to read JS stream: {}", e)),
        }
    }

    async fn parse_bytes(&mut self, _meta: FetchResultMeta, body: &[u8]) -> anyhow::Result<DummyJsDocument> {
        Ok(String::from_utf8_lossy(body).to_string())
    }
}
