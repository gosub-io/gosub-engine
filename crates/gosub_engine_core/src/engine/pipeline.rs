//! Pipeline modules for processing different asset types.
//!
//! Each module defines a trait for parsing streams and byte slices of the respective asset type.

use crate::engine::pipeline::css::{CssPipeline, CssPipelineImpl};
use crate::engine::pipeline::font::{FontPipeline, FontPipelineImpl};
use crate::engine::pipeline::html::{HtmlPipeline, HtmlPipelineImpl};
use crate::engine::pipeline::image::{ImagePipeline, ImagePipelineImpl};
use crate::engine::pipeline::js::{JsPipeline, JsPipelineImpl};
use crate::engine::types::IoChannel;
use crate::zone::ZoneId;
use gosub_interface::config::HasDocument;

pub mod css;
pub mod font;
pub mod html;
pub mod image;
pub mod js;

/// Hooks are functions that allows the router to call the correct pipeline for each type of
/// resource.
pub struct Hooks<C: HasDocument> {
    pub html: Box<dyn HtmlPipeline<C> + Send>,
    pub css: Box<dyn CssPipeline + Send>,
    pub js: Box<dyn JsPipeline + Send>,
    pub images: Box<dyn ImagePipeline + Send>,
    pub fonts: Box<dyn FontPipeline + Send>,
    // pub viewer: &'a mut dyn ViewerPipeline,
    // pub download: &'a mut dyn DownloadManager,
    // pub external: &'a mut dyn ExternalOpener,
}

impl<C: HasDocument + Send + Sync + 'static> Hooks<C>
where
    C::Document: Send + Sync,
{
    pub fn new(zone_id: ZoneId, io_tx: IoChannel) -> Self {
        Self {
            html: Box::new(HtmlPipelineImpl::<C>::new(zone_id, io_tx)),
            css: Box::new(CssPipelineImpl {}),
            js: Box::new(JsPipelineImpl {}),
            images: Box::new(ImagePipelineImpl {}),
            fonts: Box::new(FontPipelineImpl {}),
        }
    }
}
