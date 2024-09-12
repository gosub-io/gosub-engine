use ::resvg::usvg;

use gosub_html5::{node::NodeId, parser::document::DocumentHandle};
use gosub_shared::types::Result;

#[cfg(feature = "resvg")]
pub mod resvg;

pub struct SVGDocument {
    tree: usvg::Tree,
}

impl SVGDocument {
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(svg: &str) -> Result<Self> {
        let opts = usvg::Options {
            ..Default::default()
        };

        let tree = usvg::Tree::from_str(svg, &opts)?;
        Ok(Self { tree })
    }

    pub fn from_html_doc(id: NodeId, doc: DocumentHandle) -> Result<Self> {
        let str = doc.get().write_from_node(id);

        Self::from_str(&str)
    }
}
