use crate::engine::types::PeekBuf;
use crate::net::types::FetchResultMeta;
use crate::net::{stream_to_bytes, SharedBody};
use async_trait::async_trait;
use bytes::Bytes;
use std::sync::Arc;

/// A web font loaded from the network, ready to be handed to the render backend.
///
/// The raw bytes are kept intact so the renderer (via `skrifa` / `parley`) can
/// parse and rasterise the font on demand without an intermediate file path.
#[derive(Clone, Debug)]
pub struct LoadedFont {
    /// Raw font data (TTF / OTF / WOFF / WOFF2).
    pub data: Bytes,
    /// Final URL the font was fetched from (useful for caching and debugging).
    pub source_url: String,
}

#[async_trait]
pub trait FontPipeline: Send {
    async fn parse_stream(
        &mut self,
        meta: FetchResultMeta,
        peek_buf: PeekBuf,
        body: Arc<SharedBody>,
    ) -> anyhow::Result<LoadedFont>;

    async fn parse_bytes(&mut self, meta: FetchResultMeta, body: &[u8]) -> anyhow::Result<LoadedFont>;
}

pub struct FontPipelineImpl;

#[async_trait]
impl FontPipeline for FontPipelineImpl {
    async fn parse_stream(
        &mut self,
        meta: FetchResultMeta,
        peek_buf: PeekBuf,
        shared: Arc<SharedBody>,
    ) -> anyhow::Result<LoadedFont> {
        let buf = stream_to_bytes(peek_buf, shared).await?;
        self.parse_bytes(meta, buf.as_ref()).await
    }

    async fn parse_bytes(&mut self, meta: FetchResultMeta, body: &[u8]) -> anyhow::Result<LoadedFont> {
        Ok(LoadedFont {
            data: Bytes::copy_from_slice(body),
            source_url: meta.final_url.to_string(),
        })
    }
}
