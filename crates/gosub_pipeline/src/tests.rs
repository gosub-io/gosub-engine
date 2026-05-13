//! Integration tests: parse real HTML+CSS through the gosub parsers and verify
//! the pipeline's RenderTree is built correctly.

#[cfg(test)]
mod rendertree_from_engine {
    use std::sync::Arc;

    use gosub_css3::system::Css3System;
    use gosub_html5::document::document_impl::DocumentImpl;
    use gosub_html5::html_compile;
    use gosub_interface::config::{HasCssSystem, HasDocument};
    use gosub_interface::css3::CssSystem as _;
    use gosub_interface::document::Document as _;

    use crate::common::document::pipeline_doc::GosubDocumentAdapter;
    use crate::rendertree_builder::tree::RenderTree;

    // Minimal config wiring gosub_html5 + gosub_css3 together.
    #[derive(Clone, Debug, PartialEq)]
    struct Config;

    impl HasCssSystem for Config {
        type CssSystem = Css3System;
    }
    impl HasDocument for Config {
        type Document = DocumentImpl<Self>;
    }

    /// Parse HTML (with optional inline `<style>`), add the UA stylesheet, and
    /// return a RenderTree built from `GosubDocumentAdapter`.
    fn parse_to_rendertree(html: &str) -> RenderTree {
        let mut doc = html_compile::<Config>(html);

        // Add the browser UA stylesheet so default display values are available.
        let ua = Css3System::load_default_useragent_stylesheet();
        doc.add_stylesheet(ua);

        let adapter = GosubDocumentAdapter::<Config>::new(doc);
        let mut rt = RenderTree::new(Arc::new(adapter));
        rt.parse();
        rt
    }

    // -----------------------------------------------------------------------
    // Basic sanity: a minimal document produces a non-empty tree
    // -----------------------------------------------------------------------

    #[test]
    fn minimal_document_has_root() {
        let rt = parse_to_rendertree("<html><body><p>Hello</p></body></html>");
        assert!(rt.root_id.is_some(), "render tree must have a root");
        assert!(rt.count_elements() > 0, "render tree must not be empty");
    }

    // -----------------------------------------------------------------------
    // display:none must filter the node and its subtree out of the render tree
    // -----------------------------------------------------------------------

    #[test]
    fn display_none_element_is_excluded() {
        // The <div id="hidden"> and everything inside it should not appear.
        let html = r#"
            <html>
            <head>
                <style>
                    #hidden { display: none; }
                </style>
            </head>
            <body>
                <p>Visible</p>
                <div id="hidden"><span>Should be gone</span></div>
            </body>
            </html>
        "#;

        let rt = parse_to_rendertree(html);
        assert!(rt.root_id.is_some());

        // Walk every render node and confirm none map to an element that has
        // id="hidden" or is a descendant of it.
        let doc_ref = rt.doc.clone();
        for render_id in rt.arena.keys() {
            if let Some(node) = doc_ref.get_node_by_id(gosub_shared::node::NodeId::from(*render_id)) {
                use crate::common::document::node::NodeType;
                if let NodeType::Element(data) = &node.node_type {
                    let id_attr = data.attributes.get("id");
                    assert_ne!(
                        id_attr.map(|s| s.as_str()),
                        Some("hidden"),
                        "display:none element should not appear in render tree"
                    );
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Invisible structural elements (head, script, style) must be excluded
    // -----------------------------------------------------------------------

    #[test]
    fn head_and_script_are_excluded() {
        let html = r#"
            <html>
            <head><title>Test</title><style>body{color:red}</style></head>
            <body><p>Content</p></body>
            </html>
        "#;

        let rt = parse_to_rendertree(html);
        let doc_ref = rt.doc.clone();

        for render_id in rt.arena.keys() {
            if let Some(node) = doc_ref.get_node_by_id(gosub_shared::node::NodeId::from(*render_id)) {
                use crate::common::document::node::NodeType;
                if let NodeType::Element(data) = &node.node_type {
                    use cow_utils::CowUtils;
                    let tag = data.tag_name.cow_to_ascii_lowercase();
                    assert!(
                        !matches!(&*tag, "head" | "style" | "script" | "title"),
                        "invisible element <{tag}> must not appear in render tree"
                    );
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // CSS width/height values flow through to the StylePropertyList
    // -----------------------------------------------------------------------

    #[test]
    fn css_dimensions_are_extracted() {
        let html = r#"
            <html>
            <head>
                <style>
                    #box { width: 200px; height: 100px; display: block; }
                </style>
            </head>
            <body><div id="box">content</div></body>
            </html>
        "#;

        use crate::common::document::pipeline_doc::PipelineDocument;
        use crate::common::document::style::{StyleProperty, StyleValue, Unit};

        let mut doc = html_compile::<Config>(html);
        let ua = Css3System::load_default_useragent_stylesheet();
        doc.add_stylesheet(ua);

        let adapter = GosubDocumentAdapter::<Config>::new(doc);

        // Walk the gosub document to find the <div id="box"> node.
        let root = adapter.doc.root();
        let box_node_id = find_node_by_id_attr(&adapter.doc, root, "box");
        assert!(box_node_id.is_some(), "should find #box element");

        let id = box_node_id.unwrap();
        let width = adapter.get_style(id, StyleProperty::Width);
        let height = adapter.get_style(id, StyleProperty::Height);

        assert!(
            matches!(width, Some(StyleValue::Unit(w, Unit::Px)) if (w - 200.0).abs() < 0.5),
            "expected width:200px, got {width:?}"
        );
        assert!(
            matches!(height, Some(StyleValue::Unit(h, Unit::Px)) if (h - 100.0).abs() < 0.5),
            "expected height:100px, got {height:?}"
        );
    }

    // -----------------------------------------------------------------------
    // html_node_id / body_node_id resolve to the correct elements
    // -----------------------------------------------------------------------

    #[test]
    fn html_and_body_node_ids_are_found() {
        use crate::common::document::pipeline_doc::PipelineDocument;

        let html = "<html><body><p>Hi</p></body></html>";
        let doc = html_compile::<Config>(html);
        let adapter = GosubDocumentAdapter::<Config>::new(doc);

        let html_id = adapter.html_node_id();
        let body_id = adapter.body_node_id();

        assert!(html_id.is_some(), "html_node_id must resolve");
        assert!(body_id.is_some(), "body_node_id must resolve");
        assert_ne!(html_id, body_id);

        // The tag names must match.
        assert_eq!(adapter.tag_name(html_id.unwrap()), Some("html".to_string()));
        assert_eq!(adapter.tag_name(body_id.unwrap()), Some("body".to_string()));
    }

    // -----------------------------------------------------------------------
    // Helper: DFS to find the first element whose `id` attribute matches.
    // -----------------------------------------------------------------------
    fn find_node_by_id_attr(
        doc: &DocumentImpl<Config>,
        node: gosub_shared::node::NodeId,
        target_id: &str,
    ) -> Option<gosub_shared::node::NodeId> {
        use gosub_interface::document::Document as _;

        if let Some(attrs) = doc.attributes(node) {
            if attrs.get("id").map(|s| s.as_str()) == Some(target_id) {
                return Some(node);
            }
        }
        for &child in doc.children(node) {
            if let Some(found) = find_node_by_id_attr(doc, child, target_id) {
                return Some(found);
            }
        }
        None
    }
}
