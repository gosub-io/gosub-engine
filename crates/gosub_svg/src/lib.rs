use ::resvg::usvg;
use gosub_interface::config::HasDocument;
use gosub_interface::document::Document;
use gosub_shared::node::NodeId;
use gosub_shared::svg_limits::{
    xml_exceeds_limits, XmlLimit, MAX_SVG_NESTING_DEPTH, SVG_PARSE_STACK_NEEDED, SVG_PARSE_STACK_SIZE,
};
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
        image_href_resolver: usvg::ImageHrefResolver {
            resolve_data: usvg::ImageHrefResolver::default_data_resolver(),
            // usvg's default treats any other `href` as a path and reads it
            // from disk, so a document naming `/etc/passwd` would have it
            // read. `data:` or nothing, as in the render pipeline's decoder.
            resolve_string: Box::new(|_, _| None),
        },
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
        match xml_exceeds_limits(svg.as_bytes(), MAX_SVG_NESTING_DEPTH) {
            Some(XmlLimit::Depth) => {
                return Err(Error::Parse(format!(
                    "SVG nests elements deeper than the {MAX_SVG_NESTING_DEPTH} level limit"
                ))
                .into())
            }
            Some(XmlLimit::Entities) => {
                return Err(
                    Error::Parse("SVG declares its own entities, which can expand to unbounded nesting".into()).into(),
                )
            }
            None => {}
        }

        // Grow the stack rather than move to a thread; see `gosub_shared::svg_limits`. Only
        // allocates when the caller has less than `SVG_PARSE_STACK_NEEDED` left.
        let tree = stacker::maybe_grow(SVG_PARSE_STACK_NEEDED, SVG_PARSE_STACK_SIZE, || {
            usvg::Tree::from_str(svg, &svg_options()).map_err(|e| Error::Parse(e.to_string()))
        })?;

        Ok(Self { tree })
    }

    pub fn from_html_doc<C: HasDocument>(id: NodeId, doc: C::Document) -> Result<Self> {
        let str = doc.write_from_node(id);

        Self::from_str(&str)
    }
}

#[cfg(test)]
mod tests {
    use super::SVGDocument;

    /// An `<image>` naming a file is not read from disk; `data:` still works.
    #[test]
    fn an_image_href_is_never_a_file() {
        let path = std::env::temp_dir().join(format!("gosub-svg-href-{}.svg", std::process::id()));
        std::fs::write(
            &path,
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4"><rect width="4" height="4"/></svg>"#,
        )
        .unwrap();
        let page = |href: &str| {
            format!(
                r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="8" height="8"><image width="8" height="8" xlink:href="{href}"/></svg>"#
            )
        };
        let from_file = SVGDocument::from_str(&page(path.to_str().unwrap())).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert!(!from_file.tree.root().has_children(), "the file was read");

        let inline = "data:image/svg+xml;utf8,%3Csvg xmlns='http://www.w3.org/2000/svg' width='4' height='4'%3E%3Crect width='4' height='4'/%3E%3C/svg%3E";
        let from_data = SVGDocument::from_str(&page(inline)).unwrap();
        assert!(from_data.tree.root().has_children(), "control: a data: image loads");
    }
}
