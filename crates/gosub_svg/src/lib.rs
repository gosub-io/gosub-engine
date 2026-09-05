use ::resvg::usvg;
use gosub_interface::config::HasDocument;
use gosub_interface::document::Document;
use gosub_shared::node::NodeId;
use gosub_shared::svg_limits::{xml_nesting_depth_exceeds, MAX_SVG_NESTING_DEPTH, SVG_PARSE_STACK_SIZE};
use gosub_shared::types::{Error, Result};
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
    /// Parse an SVG document, depth-limited and on a stack of its own.
    ///
    /// See [`gosub_shared::svg_limits`] for why both are needed.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(svg: &str) -> Result<Self> {
        if xml_nesting_depth_exceeds(svg.as_bytes(), MAX_SVG_NESTING_DEPTH) {
            return Err(Error::Parse(format!(
                "SVG nests elements deeper than the {MAX_SVG_NESTING_DEPTH} level limit"
            ))
            .into());
        }

        // `svg_options()` is built inside the closure: `usvg::Options` holds non-`Send` resolver
        // closures. Its fontdb is a `OnceLock`-shared `Arc`, so nothing is rescanned.
        let tree = std::thread::scope(|scope| {
            let parse = std::thread::Builder::new()
                .name("svg-parse".into())
                .stack_size(SVG_PARSE_STACK_SIZE)
                .spawn_scoped(scope, || usvg::Tree::from_str(svg, &svg_options()))
                .map_err(|e| Error::Parse(format!("could not spawn SVG parse thread: {e}")))?;

            parse
                .join()
                .map_err(|_| Error::Parse("SVG parser panicked".into()))?
                .map_err(|e| Error::Parse(e.to_string()))
        })?;

        Ok(Self { tree })
    }

    pub fn from_html_doc<C: HasDocument>(id: NodeId, doc: C::Document) -> Result<Self> {
        let str = doc.write_from_node(id);

        Self::from_str(&str)
    }
}
