use crate::engine::types::PeekBuf;
use crate::net::types::FetchResultMeta;
use crate::net::{stream_to_bytes, SharedBody};
use async_trait::async_trait;
use std::sync::Arc;

pub type DummyStylesheet = String;

#[async_trait]
pub trait CssPipeline {
    async fn parse_stream(
        &mut self,
        meta: FetchResultMeta,
        peek_buf: PeekBuf,
        body: Arc<SharedBody>,
    ) -> anyhow::Result<DummyStylesheet>;

    async fn parse_bytes(&mut self, meta: FetchResultMeta, body: &[u8]) -> anyhow::Result<DummyStylesheet>;
}

pub struct CssPipelineImpl;

#[async_trait]
impl CssPipeline for CssPipelineImpl {
    async fn parse_stream(
        &mut self,
        _meta: FetchResultMeta,
        peek_buf: PeekBuf,
        shared: Arc<SharedBody>,
    ) -> anyhow::Result<DummyStylesheet> {
        // Normally, we send chunks to the CSS parser. Right now, we just collect everything
        match stream_to_bytes(peek_buf, shared).await {
            Ok(buf) => Ok(String::from_utf8_lossy(buf.as_ref()).to_string()),
            Err(e) => Err(anyhow::anyhow!("Failed to read CSS stream: {}", e)),
        }
    }

    async fn parse_bytes(&mut self, _meta: FetchResultMeta, body: &[u8]) -> anyhow::Result<DummyStylesheet> {
        Ok(String::from_utf8_lossy(body.as_ref()).to_string())
    }
}
