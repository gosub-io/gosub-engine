use ::resvg::usvg;
use gosub_interface::config::HasDocument;
use gosub_interface::document::Document;
use gosub_shared::node::NodeId;
use gosub_shared::types::Result;
use std::sync::{Arc, OnceLock};

/// Return `usvg::Options` backed by a shared fontdb that has system fonts loaded.
fn svg_options() -> usvg::Options<'static> {
    static FONTDB: OnceLock<Arc<usvg::fontdb::Database>> = OnceLock::new();
    let fontdb = Arc::clone(FONTDB.get_or_init(|| {
        let mut db = usvg::fontdb::Database::new();
        db.load_system_fonts();
        Arc::new(db)
    }));
    usvg::Options {
        fontdb,
        ..Default::default()
    }
}

pub struct SVGDocument {
    pub tree: usvg::Tree,
}

impl SVGDocument {
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(svg: &str) -> Result<Self> {
        let opts = svg_options();

        let tree = usvg::Tree::from_str(svg, &opts)?;
        Ok(Self { tree })
    }

    pub fn from_html_doc<C: HasDocument>(id: NodeId, doc: C::Document) -> Result<Self> {
        let str = doc.write_from_node(id);

        Self::from_str(&str)
    }
}
