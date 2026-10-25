//! List markers: which elements get a `::marker` box, what text it holds, and how it is styled.

use std::sync::Arc;

use gosub_css3::system::Css3System;
use gosub_html5::document::document_impl::DocumentImpl;
use gosub_html5::html_compile;
use gosub_html5::parser::Html5Parser;
use gosub_interface::config::ModuleConfiguration;
use gosub_interface::css3::CssSystem as _;
use gosub_interface::document::Document as _;
use gosub_interface::style::{Color, Display, LengthPercentage};
use gosub_shared::node::NodeId;

use crate::common::document::node::NodeType;
use crate::common::document::pipeline_doc::{is_marker_box, GosubDocumentAdapter, PipelineDocument};

#[derive(Clone, Debug, PartialEq)]
struct Config;

impl ModuleConfiguration for Config {
    type CssSystem = Css3System;
    type Document = DocumentImpl<Self>;
    type HtmlParser = Html5Parser<'static, Self>;
}

fn adapter(html: &str) -> GosubDocumentAdapter<Config> {
    let mut doc = html_compile::<Config>(html);
    doc.add_stylesheet(Css3System::load_default_useragent_stylesheet());
    GosubDocumentAdapter::<Config>::new(Arc::new(doc))
}

fn element(a: &GosubDocumentAdapter<Config>, id: &str) -> NodeId {
    fn find(a: &GosubDocumentAdapter<Config>, node: NodeId, id: &str) -> Option<NodeId> {
        if a.doc.attribute(node, "id") == Some(id) {
            return Some(node);
        }
        a.doc.children(node).iter().find_map(|&child| find(a, child, id))
    }
    find(a, a.doc.root(), id).unwrap_or_else(|| panic!("no element with id {id}"))
}

/// The marker box of the element with this `id`, if it has one.
fn marker_of(a: &GosubDocumentAdapter<Config>, id: &str) -> Option<NodeId> {
    a.children(element(a, id))
        .into_iter()
        .find(|&child| is_marker_box(child))
}

/// The text of the marker of the element with this `id`; `None` when it has no marker.
fn marker_text(a: &GosubDocumentAdapter<Config>, id: &str) -> Option<String> {
    let marker = marker_of(a, id)?;
    let text = a
        .children(marker)
        .into_iter()
        .find_map(|child| match a.get_node_by_id(child)?.node_type {
            NodeType::Text(text) => Some(text),
            _ => None,
        });
    Some(text.unwrap_or_default())
}

#[test]
fn bullets_change_with_the_nesting_depth() {
    let a = adapter(r#"<ul><li id="one">a<ul><li id="two">b<ul><li id="three">c</li></ul></li></ul></li></ul>"#);
    assert_eq!(marker_text(&a, "one").as_deref(), Some("\u{2022} "));
    assert_eq!(marker_text(&a, "two").as_deref(), Some("\u{25E6} "));
    assert_eq!(marker_text(&a, "three").as_deref(), Some("\u{25AA} "));
}

#[test]
fn an_ordered_list_counts_its_items() {
    let a = adapter(r#"<ol><li id="a">a</li><li id="b">b</li><!-- c --><p>not an item</p><li id="c">c</li></ol>"#);
    assert_eq!(marker_text(&a, "a").as_deref(), Some("1. "));
    assert_eq!(marker_text(&a, "b").as_deref(), Some("2. "));
    assert_eq!(marker_text(&a, "c").as_deref(), Some("3. "));
}

#[test]
fn start_reversed_and_value_set_the_ordinal() {
    let a = adapter(
        r#"
        <ol start="5"><li id="s1">a</li><li id="s2">b</li></ol>
        <ol reversed><li id="r1">a</li><li id="r2">b</li><li id="r3">c</li></ol>
        <ol reversed start="10"><li id="rs1">a</li><li id="rs2">b</li></ol>
        <ol><li id="v1">a</li><li id="v2" value="10">b</li><li id="v3">c</li></ol>
        "#,
    );
    let text = |id| marker_text(&a, id);
    assert_eq!(text("s1").as_deref(), Some("5. "));
    assert_eq!(text("s2").as_deref(), Some("6. "));
    assert_eq!(text("r1").as_deref(), Some("3. "));
    assert_eq!(text("r3").as_deref(), Some("1. "));
    assert_eq!(text("rs1").as_deref(), Some("10. "));
    assert_eq!(text("rs2").as_deref(), Some("9. "));
    assert_eq!(text("v1").as_deref(), Some("1. "));
    assert_eq!(text("v2").as_deref(), Some("10. "));
    assert_eq!(text("v3").as_deref(), Some("11. "));
}

#[test]
fn list_style_type_picks_the_counter_style() {
    let a = adapter(
        r#"<style>
            #roman { list-style-type: upper-roman }
            #alpha { list-style: lower-alpha inside }
            #string { list-style-type: "-> " }
        </style>
        <ol id="roman"><li>a</li><li>b</li><li>c</li><li id="iv">d</li></ol>
        <ol id="alpha"><li>a</li><li id="b">b</li></ol>
        <ul id="string"><li id="arrow">a</li></ul>"#,
    );
    assert_eq!(marker_text(&a, "iv").as_deref(), Some("IV. "));
    assert_eq!(marker_text(&a, "b").as_deref(), Some("b. "));
    assert_eq!(marker_text(&a, "arrow").as_deref(), Some("-> "));
}

#[test]
fn only_a_list_item_with_a_marker_type_gets_a_marker() {
    let a = adapter(
        r#"<style>
            nav ul { list-style: none }
            .block { display: block }
            .item { display: list-item; list-style-type: square }
        </style>
        <nav><ul><li id="plain">a</li></ul></nav>
        <ul><li id="block" class="block">b</li></ul>
        <div id="div" class="item">c</div>
        <p id="para">d</p>"#,
    );
    assert_eq!(marker_text(&a, "plain"), None);
    assert_eq!(marker_text(&a, "block"), None);
    assert_eq!(marker_text(&a, "div").as_deref(), Some("\u{25AA} "));
    assert_eq!(marker_text(&a, "para"), None);
}

#[test]
fn marker_content_replaces_the_counter() {
    let a = adapter(
        r#"<style>
            #arrow::marker { content: "\2192  " }
            #none::marker { content: none }
            #normal::marker { content: normal }
        </style>
        <ul><li id="arrow">a</li><li id="none">b</li><li id="normal">c</li></ul>"#,
    );
    assert_eq!(marker_text(&a, "arrow").as_deref(), Some("\u{2192} "));
    assert_eq!(marker_text(&a, "none"), None);
    assert_eq!(marker_text(&a, "normal").as_deref(), Some("\u{2022} "));
}

#[test]
fn the_marker_takes_its_own_style_and_stays_inline() {
    let a = adapter(
        r#"<style>
            li { color: blue }
            #red::marker { color: red; display: block }
        </style>
        <ul><li id="red">a</li><li id="blue">b</li></ul>"#,
    );
    let red = a.computed_style(marker_of(&a, "red").expect("marker"));
    assert_eq!(red.inherited.color, Color::rgba(255, 0, 0, 255));
    assert_eq!(red.box_group.display, Display::Inline);
    // `white-space: pre` from the UA sheet, so the space after the bullet is kept.
    assert!(red.inherited.white_space.preserves_spaces());

    let blue = a.computed_style(marker_of(&a, "blue").expect("marker"));
    assert_eq!(blue.inherited.color, Color::rgba(0, 0, 255, 255));
}

/// The `style` attribute styles its element, not the element's pseudo-elements: a padded item
/// used to pad its marker too, and a padded span its `::before`.
#[test]
fn the_style_attribute_stays_off_the_pseudo_elements() {
    let a = adapter(
        r#"<style>#span::before { content: "x" }</style>
        <ul><li id="item" style="padding-top: 7px; color: red">a</li></ul>
        <span id="span" style="padding-left: 9px">b</span>"#,
    );
    let marker = a.computed_style(marker_of(&a, "item").expect("marker"));
    assert_eq!(marker.padding.top, LengthPercentage::ZERO);
    // What inherits still comes down from the item.
    assert_eq!(marker.inherited.color, Color::rgba(255, 0, 0, 255));

    let before = a.children(element(&a, "span"))[0];
    assert_eq!(a.computed_style(before).padding.left, LengthPercentage::ZERO);
    assert_eq!(
        a.computed_style(element(&a, "span")).padding.left,
        LengthPercentage::Px(9.0)
    );
}

#[test]
fn the_marker_comes_before_before() {
    let a = adapter(
        r#"<style>#item::before { content: "x" }</style>
        <ol><li id="item">a</li></ol>"#,
    );
    let item = element(&a, "item");
    let children = a.children(item);
    assert!(is_marker_box(children[0]), "the first box child is the marker");
    assert!(!is_marker_box(children[1]));
    assert_eq!(a.parent(children[0]), Some(item));
}

#[test]
fn a_summary_shows_its_disclosure_triangle() {
    let a = adapter(
        r#"<details><summary id="closed">more</summary>x</details>
        <details open><summary id="open">less</summary>y</details>"#,
    );
    assert_eq!(marker_text(&a, "closed").as_deref(), Some("\u{25B8} "));
    assert_eq!(marker_text(&a, "open").as_deref(), Some("\u{25BE} "));
}

/// Lay `html` out in an 800x600 viewport and return the border box of the marker of the element
/// with `id`, and of the first piece of text in the element with `text_in`.
fn marker_and_text_boxes(html: &str, id: &str, text_in: &str) -> (crate::common::geo::Rect, crate::common::geo::Rect) {
    use crate::common::document::pipeline_doc::pseudo_owner;
    use crate::common::geo::Dimension;
    use crate::layouter::taffy::TaffyLayouter;
    use crate::layouter::{CanLayout as _, ElementContext};
    use crate::rendertree_builder::tree::RenderTree;

    let a = adapter(html);
    let item = element(&a, id);
    let text_parent = element(&a, text_in);
    let mut rt = RenderTree::new(Arc::new(a));
    rt.parse().expect("render tree");
    let tree = TaffyLayouter::new().layout(rt, Some(Dimension::new(800.0, 600.0)), 1.0);
    let marker = tree
        .arena
        .values()
        .find(|el| is_marker_box(el.dom_node_id) && pseudo_owner(el.dom_node_id) == Some(item))
        .expect("the marker is laid out");
    let text = tree
        .arena
        .values()
        .filter(|el| {
            matches!(el.context, ElementContext::Text(_))
                && pseudo_owner(el.dom_node_id).is_none()
                && tree.render_tree.doc.parent(el.dom_node_id) == Some(text_parent)
        })
        .min_by(|a, b| a.box_model.border_box.x.total_cmp(&b.box_model.border_box.x))
        .expect("the item's text is laid out");
    (marker.box_model.border_box, text.box_model.border_box)
}

#[test]
fn an_outside_marker_hangs_before_the_first_line() {
    let (marker, text) = marker_and_text_boxes(
        r#"<body style="margin: 0"><ul style="margin: 0; padding-left: 40px"><li id="item">first item</li></ul></body>"#,
        "item",
        "item",
    );
    // The text starts at the item's content edge, and the marker ends there.
    assert_eq!(text.x, 40.0, "the text is not moved by the marker");
    assert!(marker.width > 0.0, "the marker has its text's width");
    assert!(
        (marker.x + marker.width - 40.0).abs() < 0.5,
        "the marker ends at the content edge: {marker:?}"
    );
    assert!(
        (marker.y - text.y).abs() < 0.5,
        "on the first line: {marker:?} {text:?}"
    );
}

#[test]
fn an_outside_marker_hangs_beside_a_block_in_the_item() {
    let (marker, text) = marker_and_text_boxes(
        r#"<body style="margin: 0"><ol style="margin: 0; padding: 20px 0 0 40px">
            <li id="item" style="padding-top: 7px"><p id="para" style="margin: 0">two<br>lines</p></li>
        </ol></body>"#,
        "item",
        "para",
    );
    assert_eq!(text.x, 40.0);
    assert!(
        (marker.y - text.y).abs() < 0.5,
        "level with the first line: {marker:?} {text:?}"
    );
    // The item's padding is its own, not its marker's.
    assert_eq!(marker.height, text.height, "one line tall");
}

#[test]
fn an_inside_marker_starts_the_first_line() {
    let (marker, text) = marker_and_text_boxes(
        r#"<body style="margin: 0"><ul style="margin: 0; padding-left: 40px; list-style-position: inside"><li id="item">first item</li></ul></body>"#,
        "item",
        "item",
    );
    assert_eq!(marker.x, 40.0, "the marker is where the line starts");
    assert!(
        text.x >= marker.x + marker.width - 0.5,
        "the text follows it: {marker:?} {text:?}"
    );
    assert!((marker.y - text.y).abs() < 0.5);
}

#[test]
fn the_type_attribute_sets_the_style() {
    let a = adapter(
        r#"<ol type="A"><li>a</li><li id="b">b</li></ol>
        <ol type="i"><li>a</li><li id="ii">b</li><li id="override" type="1">c</li></ol>
        <ul type="square"><li id="sq">a</li></ul>
        <style>.styled { list-style-type: circle }</style>
        <ul type="square" class="styled"><li id="css">a</li></ul>"#,
    );
    assert_eq!(marker_text(&a, "b").as_deref(), Some("B. "));
    assert_eq!(marker_text(&a, "ii").as_deref(), Some("ii. "));
    assert_eq!(marker_text(&a, "override").as_deref(), Some("3. "));
    assert_eq!(marker_text(&a, "sq").as_deref(), Some("\u{25AA} "));
    // A hint loses to any author rule.
    assert_eq!(marker_text(&a, "css").as_deref(), Some("\u{25E6} "));
}
