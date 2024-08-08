use anyhow::anyhow;
use tiny_skia::Pixmap;

use gosub_html5::node::NodeId;
use gosub_html5::parser::document::DocumentHandle;
use gosub_render_backend::geo::FP;
use gosub_render_backend::svg::SvgRenderer;
use gosub_render_backend::{Image, ImageBuffer, RenderBackend};
use gosub_shared::types::Result;

use crate::SVGDocument;

pub struct Resvg;

impl<B: RenderBackend> SvgRenderer<B> for Resvg {
    type SvgDocument = SVGDocument;

    fn new(_: &mut B::WindowData<'_>) -> Self {
        Self
    }

    fn parse_external(data: String) -> Result<Self::SvgDocument> {
        SVGDocument::from_str(&data)
    }

    fn parse_internal(tree: DocumentHandle, id: NodeId) -> Result<Self::SvgDocument> {
        SVGDocument::from_html_doc(id, tree)
    }

    fn render(&mut self, doc: &SVGDocument) -> Result<ImageBuffer<B>> {
        let img: B::Image = Self::render_to_image::<B>(self, doc)?;

        Ok(ImageBuffer::Image(img))
    }
}

impl Resvg {
    pub fn render_to_image<B: RenderBackend>(&mut self, doc: &SVGDocument) -> Result<B::Image> {
        let size = doc.tree.size().to_int_size();

        let mut pixmap = Pixmap::new(size.width(), size.height())
            .ok_or_else(|| anyhow!("Failed to create pixmap"))?;

        resvg::render(
            &doc.tree,
            tiny_skia::Transform::default(),
            &mut pixmap.as_mut(),
        );

        Ok(tiny_skia_pixmap_to_img::<B>(pixmap))
    }
}

fn tiny_skia_pixmap_to_img<B: RenderBackend>(pixmap: Pixmap) -> B::Image {
    let w = pixmap.width();
    let h = pixmap.height();

    Image::new((w as FP, h as FP), pixmap.take())
}
