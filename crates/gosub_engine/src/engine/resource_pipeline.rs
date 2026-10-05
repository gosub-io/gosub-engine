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
use parking_lot::Mutex;
use std::sync::Arc;

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
/// `@font-face` web fonts, loaded as a stage of the document parse.
pub mod webfonts;

/// Resource pipeline entry points used by the router for each resource type.
pub struct ResourcePipelines<C: RenderConfiguration> {
    pub html: Box<dyn HtmlPipeline<C> + Send>,
    pub css: Box<dyn CssPipeline + Send>,
    pub js: Box<dyn JsPipeline + Send>,
    pub fonts: Box<dyn FontPipeline + Send>,
    /// `net.download.max_spool_bytes`: cap on a download body spooled before the embedder
    /// accepts it. 0 = unlimited.
    pub(crate) max_download_spool_bytes: u64,
    // pub viewer: &'a mut dyn ViewerPipeline,
    // pub download: &'a mut dyn DownloadManager,
    // pub external: &'a mut dyn ExternalOpener,
}

impl<C: RenderConfiguration> ResourcePipelines<C> {
    #[allow(clippy::too_many_arguments)] // one per pipeline setting, all set per navigation
    pub(crate) fn new(
        zone_id: ZoneId,
        tab_id: TabId,
        io_tx: IoChannel,
        accept_language: Option<String>,
        max_document_bytes: usize,
        max_download_spool_bytes: u64,
        font_system: Arc<Mutex<C::FontSystem>>,
        capture_source: bool,
        source_only: bool,
    ) -> Self {
        // A renderer process that will re-parse the document is also the only
        // process that should parse it: keep just the source here. Not for the
        // engine's own pages (`source_only` false), which may still render
        // in-process when the renderer cannot, and need a document for that.
        Self {
            max_download_spool_bytes,
            html: Box::new(
                HtmlPipelineImpl::new(
                    zone_id,
                    tab_id,
                    io_tx,
                    accept_language,
                    max_document_bytes,
                    font_system,
                    capture_source,
                )
                .source_only(source_only),
            ),
            css: Box::new(CssPipelineImpl {}),
            js: Box::new(JsPipelineImpl {}),
            fonts: Box::new(FontPipelineImpl {}),
        }
    }
}
