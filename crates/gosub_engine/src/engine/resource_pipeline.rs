//! Resource pipeline modules for processing different asset types.
//!
//! Each module defines a trait for parsing streams and byte slices of the respective asset type.

use crate::engine::resource_pipeline::css::{CssPipeline, CssPipelineImpl};
use crate::engine::resource_pipeline::font::{FontPipeline, FontPipelineImpl};
use crate::engine::resource_pipeline::html::{HtmlPipeline, HtmlPipelineImpl};
use crate::engine::resource_pipeline::image::{ImagePipeline, ImagePipelineImpl};
use crate::engine::resource_pipeline::js::{JsPipeline, JsPipelineImpl};
use crate::engine::types::IoChannel;
use crate::html::EngineConfig;
use crate::zone::ZoneId;

pub mod css;
pub mod font;
pub mod html;
pub mod image;
pub mod js;

/// Resource pipeline entry points used by the router for each resource type.
pub struct ResourcePipelines<C: EngineConfig> {
    pub html: Box<dyn HtmlPipeline<C> + Send>,
    pub css: Box<dyn CssPipeline + Send>,
    pub js: Box<dyn JsPipeline + Send>,
    pub images: Box<dyn ImagePipeline + Send>,
    pub fonts: Box<dyn FontPipeline + Send>,
    // pub viewer: &'a mut dyn ViewerPipeline,
    // pub download: &'a mut dyn DownloadManager,
    // pub external: &'a mut dyn ExternalOpener,
}

impl<C: EngineConfig> ResourcePipelines<C> {
    pub fn new(zone_id: ZoneId, io_tx: IoChannel) -> Self {
        Self {
            html: Box::new(HtmlPipelineImpl::new(zone_id, io_tx)),
            css: Box::new(CssPipelineImpl {}),
            js: Box::new(JsPipelineImpl {}),
            images: Box::new(ImagePipelineImpl {}),
            fonts: Box::new(FontPipelineImpl {}),
        }
    }
}
