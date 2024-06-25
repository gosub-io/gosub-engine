use gosub_html5::node::NodeId;
use gosub_html5::parser::document::DocumentHandle;
use gosub_shared::types::Result;

use crate::{ImageBuffer, RenderBackend};

pub trait SvgRenderer<B: RenderBackend> {
    type SvgDocument;

    fn new(wd: &mut B::WindowData<'_>) -> Self;

    fn parse_external(data: String) -> Result<Self::SvgDocument>;
    fn parse_internal(tree: DocumentHandle, id: NodeId) -> Result<Self::SvgDocument>;

    fn render(&mut self, doc: &Self::SvgDocument) -> Result<ImageBuffer<B>>;
}
