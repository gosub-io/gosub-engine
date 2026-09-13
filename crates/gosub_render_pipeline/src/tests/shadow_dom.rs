//! The flat tree: how shadow hosts, slots and slotted content are traversed once the pipeline
//! looks at a document, and what that does to style inheritance.

use std::sync::Arc;

use gosub_css3::system::Css3System;
use gosub_html5::document::document_impl::DocumentImpl;
use gosub_html5::html_compile;
use gosub_html5::parser::Html5Parser;
use gosub_interface::config::ModuleConfiguration;
use gosub_interface::css3::CssSystem as _;
use gosub_interface::document::Document as _;
use gosub_shared::node::NodeId;

use crate::common::document::pipeline_doc::{GosubDocumentAdapter, PipelineDocument};
use crate::common::document::style::StyleProperty;

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

/// The flat tree below `<body>`, one indented line per node, as the pipeline walks it.
fn flat_tree(html: &str) -> String {
    let a = adapter(html);
    let body = a.body_node_id().expect("document has a body");

    let mut out = String::new();
    dump(&a, body, 0, &mut out);
    out
}

fn dump(a: &GosubDocumentAdapter<Config>, id: NodeId, depth: usize, out: &mut String) {
    let label = match a.doc.tag_name(id) {
        Some(tag) => format!("<{tag}>"),
        None => match a.doc.text_value(id) {
            Some(text) => format!("{text:?}"),
            None => "?".to_string(),
        },
    };
    out.push_str(&format!("{}{label}\n", "  ".repeat(depth)));
    for child in a.children(id) {
        dump(a, child, depth + 1, out);
    }
}

/// The node with `id="{needle}"`, found over the *raw* DOM - including inside shadow trees - so
/// that a test can reach nodes the flat tree hides.
fn by_id(a: &GosubDocumentAdapter<Config>, needle: &str) -> NodeId {
    fn walk(doc: &DocumentImpl<Config>, id: NodeId, needle: &str) -> Option<NodeId> {
        if doc.attribute(id, "id") == Some(needle) {
            return Some(id);
        }
        if let Some(root) = doc.shadow_root(id) {
            if let Some(found) = walk(doc, root, needle) {
                return Some(found);
            }
        }
        doc.children(id).to_vec().into_iter().find_map(|c| walk(doc, c, needle))
    }
    walk(&a.doc, a.doc.root(), needle).unwrap_or_else(|| panic!("no element with id={needle}"))
}

// ── traversal ────────────────────────────────────────────────────────────────

#[test]
fn a_host_renders_its_shadow_tree_instead_of_its_children() {
    // The <p> is light DOM with nowhere to go: there is no slot, so it renders nowhere at all.
    assert_eq!(
        flat_tree("<body><div><template shadowrootmode=open><b>shadow</b></template><p>light</p></div>"),
        "<body>\n  <div>\n    <b>\n      \"shadow\"\n"
    );
}

#[test]
fn a_slot_projects_the_light_children_into_the_shadow_tree() {
    // The slot itself generates no box: the projected <p> takes its place directly.
    assert_eq!(
        flat_tree(
            "<body><div><template shadowrootmode=open><section><slot></slot></section></template>\
             <p>light</p></div>"
        ),
        "<body>\n  <div>\n    <section>\n      <p>\n        \"light\"\n"
    );
}

#[test]
fn named_slots_take_the_children_that_ask_for_them() {
    // Order comes from the slots in the shadow tree, not from the light DOM.
    assert_eq!(
        flat_tree(
            "<body><div><template shadowrootmode=open>\
             <slot name=head></slot><slot></slot><slot name=foot></slot></template>\
             <p slot=foot>f</p><p>d</p><p slot=head>h</p></div>"
        ),
        "<body>\n  <div>\n    <p>\n      \"h\"\n    <p>\n      \"d\"\n    <p>\n      \"f\"\n"
    );
}

#[test]
fn an_empty_slot_falls_back_to_its_own_children() {
    assert_eq!(
        flat_tree("<body><div><template shadowrootmode=open><slot><i>fallback</i></slot></template></div>"),
        "<body>\n  <div>\n    <i>\n      \"fallback\"\n"
    );
}

#[test]
fn fallback_content_is_dropped_as_soon_as_something_is_assigned() {
    assert_eq!(
        flat_tree(
            "<body><div><template shadowrootmode=open><slot><i>fallback</i></slot></template>\
             <p>real</p></div>"
        ),
        "<body>\n  <div>\n    <p>\n      \"real\"\n"
    );
}

#[test]
fn a_child_naming_a_slot_that_does_not_exist_is_invisible() {
    assert_eq!(
        flat_tree(
            "<body><div><template shadowrootmode=open><slot></slot></template>\
             <p>kept</p><span slot=nowhere>lost</span></div>"
        ),
        "<body>\n  <div>\n    <p>\n      \"kept\"\n"
    );
}

#[test]
fn text_children_go_to_the_default_slot() {
    // Text has no `slot` attribute, so it can only ever land in the default slot.
    assert_eq!(
        flat_tree(
            "<body><div><template shadowrootmode=open><slot name=x></slot><slot></slot></template>\
             text<em slot=x>x</em></div>"
        ),
        "<body>\n  <div>\n    <em>\n      \"x\"\n    \"text\"\n"
    );
}

#[test]
fn a_slot_the_author_gave_a_display_keeps_a_box_of_its_own() {
    // The UA sheet gives a slot `display: contents`, so by default it is spliced away. An
    // authored `display` overrides that, and the slot becomes a real box whose children are the
    // nodes projected into it - which is what lets a slot be its own flex or grid container.
    let a = adapter(
        "<body><div id=host><template shadowrootmode=open>\
         <style>slot{display:block}</style><slot id=s></slot></template><p id=p>x</p></div>",
    );

    let host = by_id(&a, "host");
    let slot = by_id(&a, "s");

    assert!(
        a.children(host).contains(&slot),
        "a slot with an authored display must survive into the flat tree"
    );
    assert_eq!(a.children(slot), vec![by_id(&a, "p")], "and project into itself");
}

#[test]
fn a_slot_styled_without_a_display_is_still_spliced_away() {
    // Padding and a border on a `display: contents` slot are inert: no box, nothing to paint.
    let a = adapter(
        "<body><div id=host><template shadowrootmode=open>\
         <style>slot{padding:40px;border:4px solid red}</style><slot id=s></slot></template>\
         <p id=p>x</p></div>",
    );

    // The shadow tree's <style> element is a child too; the render tree drops it later.
    let children = a.children(by_id(&a, "host"));
    assert!(!children.contains(&by_id(&a, "s")), "the slot must be spliced away");
    assert!(children.contains(&by_id(&a, "p")), "and replaced by what it projects");
}

#[test]
fn a_nested_host_inside_a_shadow_tree_flattens_too() {
    assert_eq!(
        flat_tree(
            "<body><div><template shadowrootmode=open>\
             <section><template shadowrootmode=open><b><slot></slot></b></template>inner</section>\
             </template></div>"
        ),
        "<body>\n  <div>\n    <section>\n      <b>\n        \"inner\"\n"
    );
}

#[test]
fn a_slot_in_a_nested_hosts_light_dom_belongs_to_the_outer_tree() {
    // The outer <slot> sits in the light DOM of <section>, so it is projected into the inner
    // tree - and it is one of the outer tree's slots, so the outer <p> flows through both.
    assert_eq!(
        flat_tree(
            "<body><div><template shadowrootmode=open>\
             <section><template shadowrootmode=open><b><slot></slot></b></template><slot></slot></section>\
             </template><p>deep</p></div>"
        ),
        "<body>\n  <div>\n    <section>\n      <b>\n        <p>\n          \"deep\"\n"
    );
}

// ── the flat tree is what styles resolve through ─────────────────────────────

#[test]
fn a_projected_nodes_parent_is_the_slot_it_landed_in() {
    let a =
        adapter("<body><div id=host><template shadowrootmode=open><slot id=s></slot></template><p id=p>x</p></div>");
    let projected = by_id(&a, "p");

    // The DOM parent is still the host; only the flat tree moves.
    assert_eq!(a.doc.parent(projected), Some(by_id(&a, "host")));
    assert_eq!(PipelineDocument::parent(&a, projected), Some(by_id(&a, "s")));
}

#[test]
fn a_shadow_trees_top_level_nodes_have_the_host_as_parent() {
    // The shadow root itself never appears - it generates no box and has no styles.
    let a = adapter("<body><div id=host><template shadowrootmode=open><b id=in>x</b></template></div>");
    assert_eq!(PipelineDocument::parent(&a, by_id(&a, "in")), Some(by_id(&a, "host")));
}

#[test]
fn the_node_view_of_a_shadow_trees_text_reports_a_real_parent() {
    // Regression (tests.gosub.io 01-declarative-basic): `get_node_by_id` used to report the raw
    // DOM parent, so a text node directly inside a shadow tree named the shadow root - which has
    // no `Node` of its own. The layouter's text path bails out when it cannot fetch the parent
    // node, so the text was silently never laid out. Element children were unaffected, which is
    // why every earlier test missed it: they all wrapped their shadow content in an element.
    let a = adapter("<body><div id=host><template shadowrootmode=open>bare text</template></div>");

    let host = by_id(&a, "host");
    let text = a.children(host)[0];

    let node = a.get_node_by_id(text).expect("text node has a Node view");
    assert_eq!(node.parent_id, Some(host));
    assert!(
        a.get_node_by_id(node.parent_id.expect("a parent")).is_some(),
        "the reported parent must itself have a Node view, or layout drops the text"
    );
}

#[test]
fn a_projected_node_inherits_from_the_slot_not_from_its_host() {
    // Both the host and the slot set `color`. The slot is the flat-tree parent, so it wins -
    // which is the whole point of flattening `parent` rather than only `children`.
    //
    // The slot's rule has to live in the shadow tree's own sheet: since CSS scoping landed, a
    // document rule cannot reach a slot. This test used to write it in the document sheet and
    // passed only because nothing stopped it.
    let a = adapter(
        "<head><style>#host{color:red}</style></head>\
         <body><div id=host><template shadowrootmode=open><style>#s{color:green}</style>\
         <slot id=s></slot></template><p id=p>x</p></div>",
    );

    let projected = a.get_style(by_id(&a, "p"), &StyleProperty::Color);
    assert_eq!(projected, a.get_style(by_id(&a, "s"), &StyleProperty::Color));
    assert_ne!(projected, a.get_style(by_id(&a, "host"), &StyleProperty::Color));
}

#[test]
fn a_shadow_tree_inherits_through_its_host() {
    let a = adapter(
        "<head><style>#host{color:blue}</style></head>\
         <body><div id=host><template shadowrootmode=open><b id=in>x</b></template></div>",
    );

    assert_eq!(
        a.get_style(by_id(&a, "in"), &StyleProperty::Color),
        a.get_style(by_id(&a, "host"), &StyleProperty::Color)
    );
}

// ── the render tree agrees ───────────────────────────────────────────────────

#[test]
fn the_render_tree_contains_no_shadow_root_and_no_slot() {
    use crate::rendertree_builder::tree::RenderTree;

    let a =
        adapter("<body><div><template shadowrootmode=open><section><slot></slot></section></template><p>x</p></div>");
    let doc = Arc::clone(&a.doc);
    let mut rt = RenderTree::new(Arc::new(a));
    rt.parse().expect("failed to build render tree");

    for render_id in rt.arena.keys() {
        let id = NodeId::from(*render_id);
        assert_ne!(doc.tag_name(id), Some("slot"), "a slot must not generate a box");
        assert!(doc.shadow_host(id).is_none(), "a shadow root must not generate a box");
    }
}
