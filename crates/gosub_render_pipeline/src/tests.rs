//! Integration tests: parse real HTML+CSS through the gosub parsers and verify
//! the pipeline's RenderTree is built correctly.

#[cfg(test)]
mod rendertree_from_engine {
    use std::sync::Arc;

    use gosub_css3::system::Css3System;
    use gosub_html5::document::document_impl::DocumentImpl;
    use gosub_html5::html_compile;
    use gosub_html5::parser::Html5Parser;
    use gosub_interface::config::ModuleConfiguration;
    use gosub_interface::css3::CssSystem as _;
    use gosub_interface::document::Document as _;

    use crate::common::document::pipeline_doc::GosubDocumentAdapter;
    use crate::rendertree_builder::tree::RenderTree;

    // Minimal config wiring gosub_html5 + gosub_css3 together.
    #[derive(Clone, Debug, PartialEq)]
    struct Config;

    impl ModuleConfiguration for Config {
        type CssSystem = Css3System;
        type Document = DocumentImpl<Self>;
        type HtmlParser = Html5Parser<'static, Self>;
    }

    /// Parse HTML (with optional inline `<style>`), add the UA stylesheet, and
    /// return a RenderTree built from `GosubDocumentAdapter`.
    fn parse_to_rendertree(html: &str) -> RenderTree {
        let mut doc = html_compile::<Config>(html);

        // Add the browser UA stylesheet so default display values are available.
        let ua = Css3System::load_default_useragent_stylesheet();
        doc.add_stylesheet(ua);

        let adapter = GosubDocumentAdapter::<Config>::new(Arc::new(doc));
        let mut rt = RenderTree::new(Arc::new(adapter));
        rt.parse().expect("failed to build render tree");
        rt
    }

    #[test]
    fn minimal_document_has_root() {
        let rt = parse_to_rendertree("<html><body><p>Hello</p></body></html>");
        assert!(rt.root_id.is_some(), "render tree must have a root");
        assert!(rt.count_elements() > 0, "render tree must not be empty");
    }

    #[test]
    fn display_none_element_is_excluded() {
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
        use crate::common::document::style::{StyleProperty, Unit, Value};

        let mut doc = html_compile::<Config>(html);
        let ua = Css3System::load_default_useragent_stylesheet();
        doc.add_stylesheet(ua);

        let adapter = GosubDocumentAdapter::<Config>::new(Arc::new(doc));

        let root = adapter.doc.root();
        let box_node_id = find_node_by_id_attr(&adapter.doc, root, "box");
        assert!(box_node_id.is_some(), "should find #box element");

        let id = box_node_id.unwrap();
        let width = adapter.get_style(id, &StyleProperty::Width);
        let height = adapter.get_style(id, &StyleProperty::Height);

        assert!(
            matches!(width, Value::Unit(w, Unit::Px) if (w - 200.0).abs() < 0.5),
            "expected width:200px, got {width:?}"
        );
        assert!(
            matches!(height, Value::Unit(h, Unit::Px) if (h - 100.0).abs() < 0.5),
            "expected height:100px, got {height:?}"
        );
    }

    #[test]
    fn letter_spacing_em_resolves_to_px_and_inherits() {
        let html = r#"
            <html>
            <head>
                <style>
                    .m { letter-spacing: 0.14em; font-size: 20px; display: block; }
                </style>
            </head>
            <body><div class="m">HELLO</div></body>
            </html>
        "#;

        use crate::common::document::node::NodeType;
        use crate::common::document::pipeline_doc::PipelineDocument;
        use crate::common::document::style::{StyleProperty, Unit, Value};

        let mut doc = html_compile::<Config>(html);
        let ua = Css3System::load_default_useragent_stylesheet();
        doc.add_stylesheet(ua);
        let adapter = GosubDocumentAdapter::<Config>::new(Arc::new(doc));

        let root = adapter.doc.root();
        let m = find_node_by_class_dfs(&adapter.doc, root, "m").expect("find .m");

        // On the element itself: 0.14em * 20px = 2.8px.
        let ls = adapter.get_style(m, &StyleProperty::LetterSpacing);
        assert!(
            matches!(ls, Value::Unit(px, Unit::Px) if (px - 2.8).abs() < 0.1),
            "expected letter-spacing 2.8px on .m, got {ls:?}"
        );

        // Inherited by the child text node ("HELLO").
        let text_child = adapter
            .children(m)
            .into_iter()
            .find(|c| matches!(adapter.get_node_by_id(*c).map(|n| n.node_type), Some(NodeType::Text(_))))
            .expect("find text child");
        let ls_text = adapter.get_style(text_child, &StyleProperty::LetterSpacing);
        assert!(
            matches!(ls_text, Value::Unit(px, Unit::Px) if (px - 2.8).abs() < 0.1),
            "expected inherited letter-spacing 2.8px on text node, got {ls_text:?}"
        );
    }

    // Regression: `line-height: 1.7` once rounded to 2.0, inflating every paragraph.
    #[test]
    fn unitless_line_height_keeps_fraction() {
        let html = r#"
            <html>
            <head>
                <style>
                    body { font-size: 17px; line-height: 1.7; }
                </style>
            </head>
            <body><section><p class="zone-intro">Gosub is in active early-stage development.</p></section></body>
            </html>
        "#;

        use crate::common::document::node::NodeType;
        use crate::common::document::pipeline_doc::PipelineDocument;
        use crate::common::document::style::{StyleProperty, Unit, Value};

        let mut doc = html_compile::<Config>(html);
        let ua = Css3System::load_default_useragent_stylesheet();
        doc.add_stylesheet(ua);
        let adapter = GosubDocumentAdapter::<Config>::new(Arc::new(doc));

        let root = adapter.doc.root();
        let p = find_node_by_class_dfs(&adapter.doc, root, "zone-intro").expect("find p");
        let text_child = adapter
            .children(p)
            .into_iter()
            .find(|c| matches!(adapter.get_node_by_id(*c).map(|n| n.node_type), Some(NodeType::Text(_))))
            .expect("find text child");

        for id in [p, text_child] {
            let fs = adapter.get_style(id, &StyleProperty::FontSize);
            assert!(
                matches!(fs, Value::Unit(px, Unit::Px) if (px - 17.0).abs() < 0.01),
                "expected font-size 17px, got {fs:?}"
            );
            let lh = adapter.get_style(id, &StyleProperty::LineHeight);
            assert!(
                matches!(lh, Value::Number(n) if (n - 1.7).abs() < 0.01),
                "expected line-height Number(1.7), got {lh:?}"
            );
        }
    }

    #[test]
    fn mix_blend_mode_reaches_element_style() {
        use crate::common::document::pipeline_doc::PipelineDocument;
        use crate::common::document::style::{lookup, StyleProperty, Value};
        use crate::painter::commands::rectangle::BlendMode;

        let html = r#"
            <html>
            <head><style>.wreck { mix-blend-mode: multiply; }</style></head>
            <body><img class="wreck" src="x.png"></body>
            </html>
        "#;

        let mut doc = html_compile::<Config>(html);
        let ua = Css3System::load_default_useragent_stylesheet();
        doc.add_stylesheet(ua);
        let adapter = GosubDocumentAdapter::<Config>::new(Arc::new(doc));

        let root = adapter.doc.root();
        let img = find_node_by_class_dfs(&adapter.doc, root, "wreck").expect("find img");

        let v = adapter.get_style(img, &StyleProperty::MixBlendMode);
        let kw = match v {
            Value::Keyword(kw) => lookup(kw),
            other => panic!("expected keyword for mix-blend-mode, got {other:?}"),
        };
        assert_eq!(kw, "multiply");
        assert_eq!(BlendMode::from_css_keyword(&kw), BlendMode::Multiply);

        // Elements without the property default to Normal.
        let body = adapter.body_node_id().expect("body");
        let v = adapter.get_style(body, &StyleProperty::MixBlendMode);
        let kw = match v {
            Value::Keyword(kw) => lookup(kw),
            other => panic!("expected keyword, got {other:?}"),
        };
        assert_eq!(BlendMode::from_css_keyword(&kw), BlendMode::Normal);
    }

    #[test]
    fn html_and_body_node_ids_are_found() {
        use crate::common::document::pipeline_doc::PipelineDocument;

        let html = "<html><body><p>Hi</p></body></html>";
        let doc = html_compile::<Config>(html);
        let adapter = GosubDocumentAdapter::<Config>::new(Arc::new(doc));

        let html_id = adapter.html_node_id();
        let body_id = adapter.body_node_id();

        assert!(html_id.is_some(), "html_node_id must resolve");
        assert!(body_id.is_some(), "body_node_id must resolve");
        assert_ne!(html_id, body_id);

        assert_eq!(adapter.tag_name(html_id.unwrap()), Some("html".to_string()));
        assert_eq!(adapter.tag_name(body_id.unwrap()), Some("body".to_string()));
    }

    #[allow(dead_code)]
    fn find_node_by_class_dfs(
        doc: &DocumentImpl<Config>,
        node: gosub_shared::node::NodeId,
        target_class: &str,
    ) -> Option<gosub_shared::node::NodeId> {
        if let Some(attrs) = doc.attributes(node) {
            if attrs.get("class").map(|s| s.as_str()) == Some(target_class) {
                return Some(node);
            }
        }
        for &child in doc.children(node) {
            if let Some(found) = find_node_by_class_dfs(doc, child, target_class) {
                return Some(found);
            }
        }
        None
    }

    // Covers the shorthand (the HN `.votearrow` case), the longhand, and an inline style.
    #[test]
    fn background_image_is_read_from_css() {
        use crate::common::document::pipeline_doc::PipelineDocument;
        use crate::common::document::style::{lookup, StyleProperty, Value};

        let html = r#"
            <html>
            <head>
                <style>
                    .votearrow { background: url(grayarrow.gif) no-repeat; }
                    #longhand  { background-image: url("pic.png"); }
                </style>
            </head>
            <body>
                <div class="votearrow">up</div>
                <div id="longhand">x</div>
                <div id="inline" style="background-image: url(inline.gif)">y</div>
                <div id="plain">z</div>
            </body>
            </html>
        "#;

        let mut doc = html_compile::<Config>(html);
        let ua = Css3System::load_default_useragent_stylesheet();
        doc.add_stylesheet(ua);
        let adapter = GosubDocumentAdapter::<Config>::new(Arc::new(doc));
        let root = adapter.doc.root();

        let url_of = |id| match adapter.get_style(id, &StyleProperty::BackgroundImage) {
            Value::Keyword(k) => lookup(k),
            other => panic!("expected keyword url, got {other:?}"),
        };

        let longhand = find_node_by_id_attr(&adapter.doc, root, "longhand").expect("find #longhand");
        assert_eq!(url_of(longhand), "pic.png", "longhand url not read");

        let votearrow = find_node_by_class_dfs(&adapter.doc, root, "votearrow").expect("find .votearrow");
        assert_eq!(url_of(votearrow), "grayarrow.gif", "shorthand url not read");

        let inline = find_node_by_id_attr(&adapter.doc, root, "inline").expect("find #inline");
        assert_eq!(url_of(inline), "inline.gif", "inline url not read");

        // An element without a background-image gets the initial value `none`.
        let plain = find_node_by_id_attr(&adapter.doc, root, "plain").expect("find #plain");
        assert_eq!(url_of(plain), "none", "plain element should be `none`");
    }

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
