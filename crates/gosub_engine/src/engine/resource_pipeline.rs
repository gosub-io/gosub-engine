//! Resource pipeline modules for processing different asset types.
//!
//! Each module defines a trait for parsing streams and byte slices of the respective asset type.

use crate::engine::resource_pipeline::css::{CssPipeline, CssPipelineImpl};
use crate::engine::resource_pipeline::font::{FontPipeline, FontPipelineImpl};
use crate::engine::resource_pipeline::html::{HtmlPipeline, HtmlPipelineImpl};
use crate::engine::resource_pipeline::js::{JsPipeline, JsPipelineImpl};
use crate::engine::types::IoChannel;
use crate::html::RenderConfiguration;
use crate::tab::TabId;
use crate::zone::ZoneId;

// async_trait expands each method with a bare #[must_use], which nightly clippy
// rejects (double_must_use) on Result-returning fns.
#[allow(clippy::double_must_use)]
pub mod css;
#[allow(clippy::double_must_use)]
pub mod font;
#[allow(clippy::double_must_use)]
pub mod html;
#[allow(clippy::double_must_use)]
pub mod js;

/// Resource pipeline entry points used by the router for each resource type.
pub struct ResourcePipelines<C: RenderConfiguration> {
    pub html: Box<dyn HtmlPipeline<C> + Send>,
    pub css: Box<dyn CssPipeline + Send>,
    pub js: Box<dyn JsPipeline + Send>,
    pub fonts: Box<dyn FontPipeline + Send>,
    // pub viewer: &'a mut dyn ViewerPipeline,
    // pub download: &'a mut dyn DownloadManager,
    // pub external: &'a mut dyn ExternalOpener,
}

impl<C: RenderConfiguration> ResourcePipelines<C> {
    pub fn new(
        zone_id: ZoneId,
        tab_id: TabId,
        io_tx: IoChannel,
        accept_language: Option<String>,
        max_document_bytes: usize,
        capture_source: bool,
    ) -> Self {
        // A renderer process that will re-parse the document is also the only
        // process that should parse it: keep just the source here.
        Self {
            html: Box::new(
                HtmlPipelineImpl::new(
                    zone_id,
                    tab_id,
                    io_tx,
                    accept_language,
                    max_document_bytes,
                    capture_source,
                )
                .source_only(capture_source),
            ),
            css: Box::new(CssPipelineImpl {}),
            js: Box::new(JsPipelineImpl {}),
            fonts: Box::new(FontPipelineImpl {}),
        }
    }
}
