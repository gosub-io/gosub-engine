use crate::engine::types::PeekBuf;
use crate::net::types::FetchResultMeta;
use crate::net::{stream_to_bytes, SharedBody};
use async_trait::async_trait;
use bytes::Buf;
use std::sync::Arc;

pub type DummyFont = String;

#[async_trait]
pub trait FontPipeline: Send {
    async fn parse_stream(
        &mut self,
        meta: FetchResultMeta,
        peek_buf: PeekBuf,
        body: Arc<SharedBody>,
    ) -> anyhow::Result<DummyFont>;

    async fn parse_bytes(&mut self, meta: FetchResultMeta, body: &[u8]) -> anyhow::Result<DummyFont>;
}

pub struct FontPipelineImpl;

#[async_trait]
impl FontPipeline for FontPipelineImpl {
    async fn parse_stream(
        &mut self,
        _meta: FetchResultMeta,
        peek_buf: PeekBuf,
        shared: Arc<SharedBody>,
    ) -> anyhow::Result<DummyFont> {
        // Normally, we send chunks to the Font parser. Right now, we just collect everything
        match stream_to_bytes(peek_buf, shared).await {
            Ok(buf) => Ok(String::from_utf8_lossy(buf.chunk()).to_string()),
            Err(e) => Err(anyhow::anyhow!("Failed to read font stream: {}", e)),
        }
    }

    async fn parse_bytes(&mut self, _meta: FetchResultMeta, body: &[u8]) -> anyhow::Result<DummyFont> {
        Ok(String::from_utf8_lossy(body.as_ref()).to_string())
    }
}
