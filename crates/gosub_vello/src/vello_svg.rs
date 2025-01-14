use gosub_interface::config::HasDocument;

use gosub_interface::render_backend::ImageBuffer;
use gosub_interface::svg::SvgRenderer;
use gosub_shared::node::NodeId;
use gosub_shared::types::{Result, Size};

use crate::VelloBackend;
use gosub_svg::SVGDocument;

pub struct VelloSVG;

impl SvgRenderer<VelloBackend> for VelloSVG {
    type SvgDocument = SVGDocument;

    fn new() -> Self {
        Self
    }

    fn parse_external(data: String) -> Result<Self::SvgDocument> {
        SVGDocument::from_str(&data)
    }

    fn parse_internal<C: HasDocument>(tree: C::Document, id: NodeId) -> Result<Self::SvgDocument> {
        SVGDocument::from_html_doc::<C>(id, tree)
    }

    fn render(&mut self, _doc: &SVGDocument) -> Result<ImageBuffer<VelloBackend>> {
        // vello_svg::render_tree(scene.inner(), &doc.tree); //TODO: too old versions that vello_svg uses

        todo!();
    }

    fn render_with_size(
        &mut self,
        _doc: &Self::SvgDocument,
        _size: Size<u32>,
    ) -> gosub_shared::types::Result<ImageBuffer<VelloBackend>> {
        todo!()
    }
}
