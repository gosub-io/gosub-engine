use gosub_html5::node::NodeId;
use gosub_html5::parser::document::DocumentHandle;
use gosub_render_backend::svg::SvgRenderer;
use gosub_render_backend::ImageBuffer;
use gosub_shared::types::Result;

use crate::render::window::WindowData;
use crate::VelloBackend;
use gosub_svg::SVGDocument;

pub struct VelloSVG;

impl SvgRenderer<VelloBackend> for VelloSVG {
    type SvgDocument = SVGDocument;

    fn new(_: &mut WindowData) -> Self {
        Self
    }

    fn parse_external(data: String) -> Result<Self::SvgDocument> {
        SVGDocument::from_str(&data)
    }

    fn parse_internal(tree: DocumentHandle, id: NodeId) -> Result<Self::SvgDocument> {
        SVGDocument::from_html_doc(id, tree)
    }

    fn render(&mut self, _doc: &SVGDocument) -> Result<ImageBuffer<VelloBackend>> {
        // vello_svg::render_tree(scene.inner(), &doc.tree); //TODO: too old versions that vello_svg uses

        todo!();
    }
}
