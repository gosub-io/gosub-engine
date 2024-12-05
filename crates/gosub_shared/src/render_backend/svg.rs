use crate::document::DocumentHandle;
use crate::node::NodeId;
use crate::traits::config::HasDocument;
use crate::types::{Result, Size};

use super::{ImageBuffer, RenderBackend};

pub trait SvgRenderer<B: RenderBackend>: Send {
    type SvgDocument;

    fn new() -> Self;

    fn parse_external(data: String) -> Result<Self::SvgDocument>;
    fn parse_internal<C: HasDocument>(tree: DocumentHandle<C>, id: NodeId) -> Result<Self::SvgDocument>;

    fn render(&mut self, doc: &Self::SvgDocument) -> Result<ImageBuffer<B>>;
    fn render_with_size(&mut self, doc: &Self::SvgDocument, size: Size<u32>) -> Result<ImageBuffer<B>>;
}
