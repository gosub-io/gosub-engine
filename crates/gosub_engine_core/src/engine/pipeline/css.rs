use crate::engine::types::PeekBuf;
use crate::net::types::FetchResultMeta;
use crate::net::{stream_to_bytes, SharedBody};
use async_trait::async_trait;
use gosub_css3::stylesheet::CssStylesheet;
use gosub_css3::Css3;
use gosub_interface::css3::CssOrigin;
use gosub_interface::parser_config::ParserConfig;
use std::sync::Arc;

#[async_trait]
pub trait CssPipeline {
    async fn parse_stream(
        &mut self,
        meta: FetchResultMeta,
        peek_buf: PeekBuf,
        body: Arc<SharedBody>,
    ) -> anyhow::Result<CssStylesheet>;

    async fn parse_bytes(&mut self, meta: FetchResultMeta, body: &[u8]) -> anyhow::Result<CssStylesheet>;
}

pub struct CssPipelineImpl;

#[async_trait]
impl CssPipeline for CssPipelineImpl {
    async fn parse_stream(
        &mut self,
        meta: FetchResultMeta,
        peek_buf: PeekBuf,
        shared: Arc<SharedBody>,
    ) -> anyhow::Result<CssStylesheet> {
        let buf = stream_to_bytes(peek_buf, shared).await?;
        self.parse_bytes(meta, buf.as_ref()).await
    }

    async fn parse_bytes(&mut self, meta: FetchResultMeta, body: &[u8]) -> anyhow::Result<CssStylesheet> {
        let source_url = meta.final_url.as_str();
        let css_str = std::str::from_utf8(body).unwrap_or_default();

        let config = ParserConfig {
            source: Some(source_url.to_string()),
            ignore_errors: true,
            match_values: true,
            ..Default::default()
        };

        Css3::parse_str(css_str, config, CssOrigin::Author, source_url)
            .map_err(|e| anyhow::anyhow!("CSS parse error: {e}"))
    }
}
