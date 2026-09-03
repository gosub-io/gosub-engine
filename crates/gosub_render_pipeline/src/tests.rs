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
        // `noscript` is here because it is hidden by a user-agent `display: none` rule rather than
        // by the render tree's hardcoded list. With scripting enabled its contents are parsed as
        // raw text, so if the element survives, that text is drawn on the page verbatim.
        let html = r#"
            <html>
            <head><title>Test</title><style>body{color:red}</style></head>
            <body><p>Content</p><noscript><img src="//example.org/x.gif"></noscript></body>
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
                        !matches!(&*tag, "head" | "style" | "script" | "title" | "noscript"),
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

    /// `left: 0; right: 0` stretches across the containing block, not across taffy's parent.
    ///
    /// Taffy does stretch a box between opposing insets, but it measures from the immediate
    /// parent. With a narrow static wrapper between the box and its positioned ancestor, that
    /// gave the wrapper's width - and the placement pass only moved the box, so the wrong width
    /// survived. The second layout pass hands taffy insets rebased onto the parent so its own
    /// algorithm produces the right size, and re-lays-out the children at that size.
    #[test]
    fn opposing_insets_stretch_across_the_containing_block() {
        use crate::common::geo::Dimension;
        use crate::layouter::taffy::TaffyLayouter;
        use crate::layouter::CanLayout;

        // 300px positioned ancestor, 100px static wrapper in between - CodeRabbit's example.
        let html = r#"
            <html><head><style>
                #cb { position: relative; margin-left: 40px; width: 300px; height: 200px; }
                #wrap { width: 100px; }
                #target { position: absolute; left: 0; right: 0; height: 10px; }
            </style></head>
            <body style="margin:0">
                <div id="cb"><div id="wrap"><div id="target"></div></div></div>
            </body></html>
        "#;

        let mut doc = html_compile::<Config>(html);
        doc.add_stylesheet(Css3System::load_default_useragent_stylesheet());
        let adapter = GosubDocumentAdapter::<Config>::new(Arc::new(doc));
        let root = adapter.doc.root();
        let target_dom = find_node_by_id_attr(&adapter.doc, root, "target").expect("#target");

        let mut render_tree = RenderTree::new(Arc::new(adapter));
        render_tree.parse().expect("render tree");
        let layout_tree = TaffyLayouter::new().layout(render_tree, Some(Dimension::new(800.0, 600.0)), 1.0);

        let mb = layout_tree
            .arena
            .values()
            .find(|el| el.dom_node_id == target_dom)
            .expect("#target in the layout tree")
            .box_model
            .margin_box;

        assert!(
            (mb.width - 300.0).abs() < 1.0,
            "expected the box to span the 300px containing block, got width {} (the 100px wrapper?)",
            mb.width
        );
        assert!(
            (mb.x - 40.0).abs() < 1.0,
            "expected x ~40 (the containing block's left edge), got {}",
            mb.x
        );
    }

    /// The common shape - an absolute child directly inside its positioned ancestor - must still
    /// come out right, and is the case the second pass deliberately skips.
    #[test]
    fn opposing_insets_with_the_parent_as_containing_block() {
        use crate::common::geo::Dimension;
        use crate::layouter::taffy::TaffyLayouter;
        use crate::layouter::CanLayout;

        let html = r#"
            <html><head><style>
                #cb { position: relative; margin-left: 40px; width: 300px; height: 200px; }
                #target { position: absolute; left: 0; right: 0; height: 10px; }
            </style></head>
            <body style="margin:0"><div id="cb"><div id="target"></div></div></body></html>
        "#;

        let mut doc = html_compile::<Config>(html);
        doc.add_stylesheet(Css3System::load_default_useragent_stylesheet());
        let adapter = GosubDocumentAdapter::<Config>::new(Arc::new(doc));
        let root = adapter.doc.root();
        let target_dom = find_node_by_id_attr(&adapter.doc, root, "target").expect("#target");

        let mut render_tree = RenderTree::new(Arc::new(adapter));
        render_tree.parse().expect("render tree");
        let layout_tree = TaffyLayouter::new().layout(render_tree, Some(Dimension::new(800.0, 600.0)), 1.0);

        let mb = layout_tree
            .arena
            .values()
            .find(|el| el.dom_node_id == target_dom)
            .expect("#target in the layout tree")
            .box_model
            .margin_box;
        assert!((mb.width - 300.0).abs() < 1.0, "expected width ~300, got {}", mb.width);
        assert!((mb.x - 40.0).abs() < 1.0, "expected x ~40, got {}", mb.x);
    }

    /// The initial containing block sits at the canvas origin, not inside the root's padding.
    ///
    /// It used to be anchored on the root element's *content* box, so any padding on the root
    /// pushed it inwards and `top: 0; left: 0` on an unanchored absolute box - or on anything
    /// `fixed` - missed the corner by exactly that padding.
    #[test]
    fn initial_containing_block_is_anchored_at_the_origin() {
        use crate::common::geo::Dimension;
        use crate::layouter::taffy::TaffyLayouter;
        use crate::layouter::CanLayout;

        // Padding on the root, and no positioned ancestor above `#pinned`.
        let html = r#"
            <html><head><style>
                html { padding: 20px; }
                #pinned { position: absolute; left: 0; top: 0; width: 50px; height: 10px; }
            </style></head>
            <body style="margin:0"><div id="pinned"></div></body></html>
        "#;

        let mut doc = html_compile::<Config>(html);
        doc.add_stylesheet(Css3System::load_default_useragent_stylesheet());
        let adapter = GosubDocumentAdapter::<Config>::new(Arc::new(doc));
        let root = adapter.doc.root();
        let pinned_dom = find_node_by_id_attr(&adapter.doc, root, "pinned").expect("#pinned");

        let mut render_tree = RenderTree::new(Arc::new(adapter));
        render_tree.parse().expect("render tree");
        let layout_tree = TaffyLayouter::new().layout(render_tree, Some(Dimension::new(800.0, 600.0)), 1.0);

        let pinned = layout_tree
            .arena
            .values()
            .find(|el| el.dom_node_id == pinned_dom)
            .expect("#pinned in the layout tree");
        let mb = pinned.box_model.margin_box;
        assert!(
            mb.x.abs() < 0.5 && mb.y.abs() < 0.5,
            "`top: 0; left: 0` with no positioned ancestor should reach the canvas corner, got ({}, {})",
            mb.x,
            mb.y
        );
    }

    /// A font-relative inset must place the box, not be discarded as `auto`.
    ///
    /// The converter feeding taffy resolves `em`/`rem`, but the absolute-positioning pass read
    /// the raw value and matched only `px` and `%`. A box whose only specified side was an `em`
    /// inset was therefore treated as `auto` on that axis and left wherever taffy had put it,
    /// rather than placed against its containing block.
    #[test]
    fn font_relative_insets_are_honoured() {
        use crate::common::geo::Dimension;
        use crate::layouter::taffy::TaffyLayouter;
        use crate::layouter::CanLayout;

        /// x of `#target`'s margin box, laid out at 800x600.
        fn target_x(html: &str) -> f64 {
            let mut doc = html_compile::<Config>(html);
            doc.add_stylesheet(Css3System::load_default_useragent_stylesheet());
            let adapter = GosubDocumentAdapter::<Config>::new(Arc::new(doc));
            let root = adapter.doc.root();
            let target_dom = find_node_by_id_attr(&adapter.doc, root, "target").expect("#target");

            let mut render_tree = RenderTree::new(Arc::new(adapter));
            render_tree.parse().expect("render tree");
            let layout_tree = TaffyLayouter::new().layout(render_tree, Some(Dimension::new(800.0, 600.0)), 1.0);
            layout_tree
                .arena
                .values()
                .find(|el| el.dom_node_id == target_dom)
                .expect("#target in the layout tree")
                .box_model
                .margin_box
                .x
        }

        // The static `#wrap` in between is what makes this observable: taffy places an absolute
        // child against its *immediate parent*, so it puts `#target` at 150 + 64, while CSS
        // measures from `#cb` and wants 100 + 64. Dropping the inset left taffy's answer standing.
        // Without the wrapper the two agree and the bug hides.
        // 4em at the default 16px font size; `left: 64px` is the same distance spelled in px.
        let page = |left: &str| {
            format!(
                r#"<html><head><style>
                    #cb {{ position: relative; margin-left: 100px; width: 400px; height: 200px; }}
                    #wrap {{ margin-left: 50px; }}
                    #target {{ position: absolute; left: {left}; width: 50px; height: 10px; }}
                </style></head>
                <body style="margin:0">
                    <div id="cb"><div id="wrap"><div id="target"></div></div></div>
                </body></html>"#
            )
        };

        let em_x = target_x(&page("4em"));
        let px_x = target_x(&page("64px"));
        assert!(
            (em_x - px_x).abs() < 0.5,
            "`left: 4em` should place identically to `left: 64px`, got {em_x} vs {px_x}"
        );
        assert!(
            (em_x - 164.0).abs() < 1.0,
            "expected x ~164 (containing block at 100px + 4em), got {em_x}"
        );
    }

    /// With no viewport, the initial containing block comes from the root's settled size.
    ///
    /// `root_dimension` is zero until the layout pass publishes it, and that used to happen
    /// *after* the absolute-positioning pass ran - so the fallback containing block was 0x0,
    /// percentage insets resolved to zero and `right`/`bottom` placed boxes at negative offsets.
    #[test]
    fn absolute_placement_without_a_viewport_uses_the_root_size() {
        use crate::layouter::taffy::TaffyLayouter;
        use crate::layouter::CanLayout;

        // No positioned ancestor anywhere, so `#pinned` measures against the *initial*
        // containing block - the fallback this test is about. The in-flow sibling is what gives
        // the root a width to fall back to; without a viewport its size is content-driven.
        let html = r#"
            <html><head><style>
                #pinned { position: absolute; right: 0; top: 0; width: 50px; height: 10px; }
            </style></head>
            <body style="margin:0">
                <div style="width:400px;height:20px"></div>
                <div id="pinned"></div>
            </body></html>
        "#;

        let mut doc = html_compile::<Config>(html);
        doc.add_stylesheet(Css3System::load_default_useragent_stylesheet());
        let adapter = GosubDocumentAdapter::<Config>::new(Arc::new(doc));
        let root = adapter.doc.root();
        let pinned_dom = find_node_by_id_attr(&adapter.doc, root, "pinned").expect("#pinned");

        let mut render_tree = RenderTree::new(Arc::new(adapter));
        render_tree.parse().expect("render tree");
        // No viewport: the initial containing block has to fall back to the root's own size.
        let layout_tree = TaffyLayouter::new().layout(render_tree, None, 1.0);

        assert!(
            layout_tree.root_dimension.width > 0.0,
            "the root's settled size must be published before it is used"
        );

        let pinned = layout_tree
            .arena
            .values()
            .find(|el| el.dom_node_id == pinned_dom)
            .expect("#pinned in the layout tree");
        // `right: 0` puts the box's right edge on the containing block's right edge, so its
        // left edge lands at (containing block width - 50). With the zero fallback that came out
        // at -50: flush against nothing, off the left of the canvas.
        let expected = layout_tree.root_dimension.width - 50.0;
        assert!(
            pinned.box_model.margin_box.x >= 0.0,
            "right-edge placement produced a negative offset: x = {}",
            pinned.box_model.margin_box.x
        );
        assert!(
            (pinned.box_model.margin_box.x - expected).abs() < 1.0,
            "expected x ~{expected} (root width {} minus the 50px box), got {}",
            layout_tree.root_dimension.width,
            pinned.box_model.margin_box.x
        );
    }

    /// A promoted layer whose element has a collapsed margin box must still get tiles.
    ///
    /// Element-to-tile assignment unions the margin and border boxes, but the layer's tile grid
    /// was still bounded by margin boxes alone - and skipped any element with zero area. A
    /// negative margin large enough to collapse the margin box (the `margin-left: -320px` float
    /// the union was added for) therefore produced a layer with no tiles at all, so the union
    /// had nothing to select and the element's background vanished.
    #[test]
    fn collapsed_margin_box_still_gets_tiles() {
        use crate::common::geo::Dimension;
        use crate::layering::layer::LayerList;
        use crate::layouter::taffy::TaffyLayouter;
        use crate::layouter::CanLayout;
        use crate::tiler::TileList;

        // `opacity` promotes the div to its own layer, so it goes through the bounds computation
        // rather than layer 0's full-page coverage. `margin-left: -320px` against a 320px width
        // leaves a zero-width margin box while the border box keeps its 320px.
        let html = r#"
            <html><head><style>
                #ghost {
                    display: block; width: 320px; height: 100px;
                    margin-left: -320px; opacity: 0.5; background-color: #ff0000;
                }
            </style></head>
            <body style="margin:0"><div style="padding-left:400px"><div id="ghost"></div></div></body></html>
        "#;

        let mut doc = html_compile::<Config>(html);
        doc.add_stylesheet(Css3System::load_default_useragent_stylesheet());
        let adapter = GosubDocumentAdapter::<Config>::new(Arc::new(doc));
        let root = adapter.doc.root();
        let ghost_dom = find_node_by_id_attr(&adapter.doc, root, "ghost").expect("#ghost");

        let mut render_tree = RenderTree::new(Arc::new(adapter));
        render_tree.parse().expect("render tree");
        let layout_tree = TaffyLayouter::new().layout(render_tree, Some(Dimension::new(800.0, 600.0)), 1.0);

        // Confirm the setup really does collapse the margin box - if a layout change ever stops
        // reproducing that, this test should say so rather than pass hollowly.
        let ghost = layout_tree
            .arena
            .iter()
            .find(|(_, el)| el.dom_node_id == ghost_dom)
            .map(|(id, el)| (*id, el.box_model))
            .expect("#ghost in the layout tree");
        assert!(
            ghost.1.margin_box.width <= 0.0,
            "the negative margin should collapse the margin box, got {}",
            ghost.1.margin_box.width
        );
        assert!(ghost.1.border_box.width > 0.0, "the border box should keep its width");

        let layer_list = LayerList::new(layout_tree);
        // More than one layer means the div really was promoted; layer 0 gets full-page coverage
        // and would bypass the bounds computation this test is about.
        assert!(
            layer_list.layer_ids.read().len() > 1,
            "the div should have been promoted to its own layer"
        );

        let mut tile_list = TileList::new(layer_list, Dimension::new(256.0, 256.0));
        tile_list.generate();

        assert!(
            !tile_list.get_tiles_for_element(ghost.0).is_empty(),
            "#ghost was assigned to no tile, so nothing paints its background"
        );
    }

    /// Floating a flex container must not turn it into a block container.
    ///
    /// CSS blockification (Display §2.7) only maps *inline-level* boxes to their block-level
    /// equivalent - `inline-flex` becomes `flex`, not `block`, and a box that is already
    /// block-level is untouched. Forcing `Display::Block` on every float laid a floated flex
    /// container's children out stacked instead of in a row.
    #[test]
    fn floated_flex_container_keeps_its_flex_children() {
        use crate::common::geo::{Dimension, Rect};
        use crate::layouter::taffy::TaffyLayouter;
        use crate::layouter::CanLayout;

        /// Lay `html` out at 800x600 and return the margin boxes of `#row`'s children.
        fn child_boxes(html: &str) -> Vec<Rect> {
            let mut doc = html_compile::<Config>(html);
            doc.add_stylesheet(Css3System::load_default_useragent_stylesheet());
            let adapter = GosubDocumentAdapter::<Config>::new(Arc::new(doc));
            let root = adapter.doc.root();
            let row_dom = find_node_by_id_attr(&adapter.doc, root, "row").expect("#row");

            let mut render_tree = RenderTree::new(Arc::new(adapter));
            render_tree.parse().expect("render tree");
            let layout_tree = TaffyLayouter::new().layout(render_tree, Some(Dimension::new(800.0, 600.0)), 1.0);

            let row = layout_tree
                .arena
                .values()
                .find(|el| el.dom_node_id == row_dom)
                .expect("#row in the layout tree");
            row.children
                .iter()
                .filter_map(|id| layout_tree.get_node_by_id(*id))
                .map(|el| el.box_model.margin_box)
                .collect()
        }

        let html = r#"
            <html><head><style>
                #row { display: flex; float: left; }
                #row > div { width: 50px; height: 20px; }
            </style></head>
            <body style="margin:0">
                <div id="row"><div id="a"></div><div id="b"></div></div>
            </body></html>
        "#;

        let boxes = child_boxes(html);
        assert_eq!(boxes.len(), 2, "expected the two flex items");
        // Side by side (flex row), not stacked (block flow).
        assert!(
            (boxes[0].y - boxes[1].y).abs() < 0.5,
            "floated flex children should share a row, got y = {} and {}",
            boxes[0].y,
            boxes[1].y
        );
        assert!(
            boxes[1].x > boxes[0].x + 1.0,
            "the second flex item should sit right of the first, got x = {} and {}",
            boxes[0].x,
            boxes[1].x
        );
    }
}
