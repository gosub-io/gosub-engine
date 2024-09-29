use ::resvg::usvg;
use gosub_shared::document::DocumentHandle;
use gosub_shared::node::NodeId;
use gosub_shared::traits::css3::CssSystem;
use gosub_shared::traits::document::Document;
use gosub_shared::types::Result;

#[cfg(feature = "resvg")]
pub mod resvg;

pub struct SVGDocument {
    tree: usvg::Tree,
}

impl SVGDocument {
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(svg: &str) -> Result<Self> {
        let opts = usvg::Options { ..Default::default() };

        let tree = usvg::Tree::from_str(svg, &opts)?;
        Ok(Self { tree })
    }

    pub fn from_html_doc<D: Document<C>, C: CssSystem>(id: NodeId, doc: DocumentHandle<D, C>) -> Result<Self> {
        let doc = doc.get();

        let str = doc.write_from_node(id);

        Self::from_str(&str)
    }
}
