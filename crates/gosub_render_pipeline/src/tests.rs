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

    /// Width of the element with `id="{id_attr}"`, in px, after the full style path.
    fn width_px_of(html: &str, id_attr: &str) -> f32 {
        use crate::common::document::pipeline_doc::PipelineDocument;
        use crate::common::document::style::{StyleProperty, Unit, Value};

        let mut doc = html_compile::<Config>(html);
        doc.add_stylesheet(Css3System::load_default_useragent_stylesheet());
        let adapter = GosubDocumentAdapter::<Config>::new(Arc::new(doc));
        let root = adapter.doc.root();
        let id = find_node_by_id_attr(&adapter.doc, root, id_attr).expect("element not found");
        match adapter.get_style(id, &StyleProperty::Width) {
            Value::Unit(w, Unit::Px) => w,
            other => panic!("expected a px width, got {other:?}"),
        }
    }

    #[test]
    fn rule_takes_the_highest_specificity_of_its_matching_selectors() {
        // `.item, #target` matches twice; the rule must cascade with the id's specificity,
        // so the later class-only rule does not win.
        let html = r#"
            <html><head><style>
                .item, #target { width: 200px; display: block; }
                .other { width: 100px; }
            </style></head>
            <body><div id="target" class="item other">x</div></body></html>
        "#;
        let w = width_px_of(html, "target");
        assert!((w - 200.0).abs() < 0.5, "expected 200.0px, got {w}");
    }

    #[test]
    fn custom_properties_cascade_by_importance_and_specificity() {
        // A later, plain declaration must not override an earlier `!important` one, and a
        // class must beat a type selector regardless of order.
        let html = r#"
            <html><head><style>
                .theme { --w: 200px !important; }
                div { --w: 50px; }
                .late { --w: 100px; }
                #target { width: var(--w); display: block; }
            </style></head>
            <body><div id="target" class="theme late">x</div></body></html>
        "#;
        let w = width_px_of(html, "target");
        assert!((w - 200.0).abs() < 0.5, "expected 200.0px, got {w}");

        let html = r#"
            <html><head><style>
                .theme { --w: 200px; }
                div { --w: 50px; }
                #target { width: var(--w); display: block; }
            </style></head>
            <body><div id="target" class="theme">x</div></body></html>
        "#;
        let w = width_px_of(html, "target");
        assert!((w - 200.0).abs() < 0.5, "expected 200.0px, got {w}");
    }

    #[test]
    fn custom_properties_inherit_from_ancestors() {
        let html = r#"
            <html><head><style>
                body { --w: 300px; }
                .mid { --w: 120px; }
                #target { width: var(--w); display: block; }
                #plain { width: var(--w); display: block; }
            </style></head>
            <body>
                <div class="mid"><div><span id="target">x</span></div></div>
                <div id="plain">y</div>
            </body></html>
        "#;
        let w = width_px_of(html, "target");
        assert!((w - 120.0).abs() < 0.5, "expected 120.0px, got {w}");
        let w = width_px_of(html, "plain");
        assert!((w - 300.0).abs() < 0.5, "expected 300.0px, got {w}");
    }

    #[test]
    fn inline_custom_properties_join_the_scope() {
        // Same element, a descendant, and a pseudo-element all resolve `var()` against the
        // `style` attribute, which outranks any stylesheet rule.
        let html = r#"
            <html><head><style>
                #target { --w: 50px; }
                #target, #child, #host { width: var(--w); display: block; }
                #host::before { content: "x"; display: block; width: var(--w); }
            </style></head>
            <body>
                <div id="target" style="--w: 200px">x</div>
                <div style="--w: 120px"><div id="child">y</div></div>
                <div id="host" style="--w: 90px">z</div>
            </body></html>
        "#;
        let w = width_px_of(html, "target");
        assert!((w - 200.0).abs() < 0.5, "expected 200px, got {w}");
        let w = width_px_of(html, "child");
        assert!((w - 120.0).abs() < 0.5, "expected 120px, got {w}");

        use crate::common::document::pipeline_doc::PipelineDocument;
        use crate::common::document::style::{StyleProperty, Unit, Value};
        let mut doc = html_compile::<Config>(html);
        doc.add_stylesheet(Css3System::load_default_useragent_stylesheet());
        let adapter = GosubDocumentAdapter::<Config>::new(Arc::new(doc));
        let root = adapter.doc.root();
        let host = find_node_by_id_attr(&adapter.doc, root, "host").expect("host");
        let before = adapter.children(host)[0];
        match adapter.get_style(before, &StyleProperty::Width) {
            Value::Unit(w, Unit::Px) => assert!((w - 90.0).abs() < 0.5, "expected 90px, got {w}"),
            other => panic!("expected a px width on ::before, got {other:?}"),
        }
    }

    /// Resolve `#target`'s width with the given viewport installed as the media environment.
    /// Each test runs on its own thread, so the thread-local environment does not leak.
    fn width_px_at_viewport(html: &str, width: f32, height: f32) -> f32 {
        gosub_css3::media_query::set_media_environment(gosub_css3::media_query::MediaEnvironment {
            width,
            height,
            device_width: width,
            device_height: height,
            ..Default::default()
        });
        width_px_of(html, "target")
    }

    /// The headline case: a mobile-first stylesheet whose desktop rules live in a `@media`
    /// block. Before media evaluation existed those rules were dropped when the stylesheet
    /// was built, so the desktop layout could never appear at any window size.
    #[test]
    fn media_block_applies_only_above_its_breakpoint() {
        let html = r#"
            <html><head><style>
                #target { width: 100px; display: block; }
                @media (min-width: 768px) {
                    #target { width: 300px; }
                }
            </style></head>
            <body><div id="target">x</div></body></html>
        "#;

        let narrow = width_px_at_viewport(html, 500.0, 800.0);
        assert!(
            (narrow - 100.0).abs() < 0.5,
            "below the breakpoint: expected 100px, got {narrow}"
        );

        let wide = width_px_at_viewport(html, 1024.0, 800.0);
        assert!(
            (wide - 300.0).abs() < 0.5,
            "above the breakpoint: expected 300px, got {wide}"
        );
    }

    /// A rule inside a matching `@media` block cascades by its own specificity and source
    /// position - the block itself adds nothing. Here a later, equally specific rule outside
    /// the block must win even though the media condition holds.
    #[test]
    fn media_block_adds_no_specificity() {
        let html = r#"
            <html><head><style>
                @media (min-width: 768px) {
                    #target { width: 300px; display: block; }
                }
                #target { width: 250px; }
            </style></head>
            <body><div id="target">x</div></body></html>
        "#;

        let w = width_px_at_viewport(html, 1024.0, 800.0);
        assert!(
            (w - 250.0).abs() < 0.5,
            "the later rule should win: expected 250px, got {w}"
        );
    }

    /// Nested blocks must both hold, and the rules inside a `@media` still flatten out of an
    /// enclosing `@layer`.
    #[test]
    fn nested_and_layered_media_blocks() {
        let html = r#"
            <html><head><style>
                #target { width: 100px; display: block; }
                @media (min-width: 700px) {
                    @media (max-width: 900px) {
                        #target { width: 200px; }
                    }
                }
                @layer desktop {
                    @media (min-width: 1200px) {
                        #target { width: 400px; }
                    }
                }
            </style></head>
            <body><div id="target">x</div></body></html>
        "#;

        // Only the inner range matches.
        let inside = width_px_at_viewport(html, 800.0, 600.0);
        assert!(
            (inside - 200.0).abs() < 0.5,
            "inside both bounds: expected 200px, got {inside}"
        );

        // Outside the nested range and below the layered one.
        let between = width_px_at_viewport(html, 1000.0, 600.0);
        assert!(
            (between - 100.0).abs() < 0.5,
            "between the blocks: expected 100px, got {between}"
        );

        // The rule inside `@layer` + `@media` is reachable.
        let widest = width_px_at_viewport(html, 1400.0, 600.0);
        assert!(
            (widest - 400.0).abs() < 0.5,
            "layered media block: expected 400px, got {widest}"
        );
    }

    /// Non-length features reach the cascade too, and read the environment rather than the
    /// viewport.
    #[test]
    fn prefers_color_scheme_selects_a_rule() {
        use gosub_css3::media_query::{ColorScheme, MediaEnvironment};

        let html = r#"
            <html><head><style>
                #target { width: 100px; display: block; }
                @media (prefers-color-scheme: dark) {
                    #target { width: 300px; }
                }
            </style></head>
            <body><div id="target">x</div></body></html>
        "#;

        gosub_css3::media_query::set_media_environment(MediaEnvironment {
            color_scheme: ColorScheme::Dark,
            ..Default::default()
        });
        let dark = width_px_of(html, "target");
        assert!((dark - 300.0).abs() < 0.5, "dark scheme: expected 300px, got {dark}");

        gosub_css3::media_query::set_media_environment(MediaEnvironment {
            color_scheme: ColorScheme::Light,
            ..Default::default()
        });
        let light = width_px_of(html, "target");
        assert!((light - 100.0).abs() < 0.5, "light scheme: expected 100px, got {light}");
    }

    /// `print`-only rules must not reach a screen render.
    #[test]
    fn print_only_rules_are_inert_on_screen() {
        let html = r#"
            <html><head><style>
                #target { width: 100px; display: block; }
                @media print {
                    #target { width: 999px; }
                }
            </style></head>
            <body><div id="target">x</div></body></html>
        "#;

        let w = width_px_at_viewport(html, 1024.0, 800.0);
        assert!(
            (w - 100.0).abs() < 0.5,
            "print rules must not apply: expected 100px, got {w}"
        );
    }
}
