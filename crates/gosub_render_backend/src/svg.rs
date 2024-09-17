use gosub_shared::document::DocumentHandle;
use gosub_shared::node::NodeId;
use gosub_shared::traits::css3::CssSystem;
use gosub_shared::traits::document::Document;
use gosub_shared::types::{Result, Size};

use crate::{ImageBuffer, RenderBackend};

pub trait SvgRenderer<B: RenderBackend>: Send {
    type SvgDocument;

    fn new() -> Self;

    fn parse_external(data: String) -> Result<Self::SvgDocument>;
    fn parse_internal<D: Document<C>, C: CssSystem>(
        tree: DocumentHandle<D, C>,
        id: NodeId,
    ) -> Result<Self::SvgDocument>;

    fn render(&mut self, doc: &Self::SvgDocument) -> Result<ImageBuffer<B>>;
    fn render_with_size(&mut self, doc: &Self::SvgDocument, size: Size<u32>) -> Result<ImageBuffer<B>>;
}
