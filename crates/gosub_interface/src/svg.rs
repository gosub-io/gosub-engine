use crate::config::HasDocument;
use crate::document_handle::DocumentHandle;
use crate::render_backend::{ImageBuffer, RenderBackend};
use gosub_shared::node::NodeId;
use gosub_shared::types::{Result, Size};

pub trait SvgRenderer<B: RenderBackend>: Send {
    type SvgDocument;

    fn new() -> Self;

    fn parse_external(data: String) -> Result<Self::SvgDocument>;
    fn parse_internal<C: HasDocument>(tree: DocumentHandle<C>, id: NodeId) -> Result<Self::SvgDocument>;

    fn render(&mut self, doc: &Self::SvgDocument) -> Result<ImageBuffer<B>>;
    fn render_with_size(&mut self, doc: &Self::SvgDocument, size: Size<u32>) -> Result<ImageBuffer<B>>;
}
