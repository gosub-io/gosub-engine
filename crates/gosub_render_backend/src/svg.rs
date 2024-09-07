use gosub_html5::node::NodeId;
use gosub_html5::parser::document::DocumentHandle;
use gosub_shared::types::{Result, Size};

use crate::{ImageBuffer, RenderBackend};

pub trait SvgRenderer<B: RenderBackend> {
    type SvgDocument;

    fn new() -> Self;

    fn parse_external(data: String) -> Result<Self::SvgDocument>;
    fn parse_internal(tree: DocumentHandle, id: NodeId) -> Result<Self::SvgDocument>;

    fn render(&mut self, doc: &Self::SvgDocument) -> Result<ImageBuffer<B>>;
    fn render_with_size(
        &mut self,
        doc: &Self::SvgDocument,
        size: Size<u32>,
    ) -> Result<ImageBuffer<B>>;
}
