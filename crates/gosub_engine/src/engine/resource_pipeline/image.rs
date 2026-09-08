use crate::engine::types::PeekBuf;
use crate::net::types::FetchResultMeta;
use crate::net::{stream_to_bytes, SharedBody};
use async_trait::async_trait;
use image::ImageReader;
use std::io::Cursor;
use std::sync::Arc;

#[async_trait]
pub trait ImagePipeline {
    async fn parse_stream(
        &mut self,
        meta: FetchResultMeta,
        peek_buf: PeekBuf,
        body: Arc<SharedBody>,
    ) -> anyhow::Result<image::DynamicImage>;

    async fn parse_bytes(&mut self, meta: FetchResultMeta, body: &[u8]) -> anyhow::Result<image::DynamicImage>;
}

pub struct ImagePipelineImpl;

#[async_trait]
impl ImagePipeline for ImagePipelineImpl {
    async fn parse_stream(
        &mut self,
        meta: FetchResultMeta,
        peek_buf: PeekBuf,
        shared: Arc<SharedBody>,
    ) -> anyhow::Result<image::DynamicImage> {
        // Normally, we send chunks to the image decoder. Right now, we just collect everything
        let buf = stream_to_bytes(peek_buf, shared)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to read image stream: {}", e))?;

        // The guard starts here, not above: draining the body is transfer, and folding it
        // into decode.image would make the counter track the network the way html5.parse
        // used to.
        let _t = gosub_shared::timing_guard!("decode.image", meta.final_url.as_str());
        ImageReader::new(Cursor::new(buf))
            .with_guessed_format()?
            .decode()
            .map_err(|e| anyhow::anyhow!("Failed to decode image: {}", e))
    }

    async fn parse_bytes(&mut self, meta: FetchResultMeta, body: &[u8]) -> anyhow::Result<image::DynamicImage> {
        let _t = gosub_shared::timing_guard!("decode.image", meta.final_url.as_str());
        ImageReader::new(Cursor::new(body))
            .with_guessed_format()?
            .decode()
            .map_err(|e| anyhow::anyhow!("Failed to decode image: {}", e))
    }
}
