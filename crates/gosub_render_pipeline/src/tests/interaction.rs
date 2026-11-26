//! The user-interaction pseudo-classes, matched through the snapshot the adapter is handed.

use std::sync::Arc;

use gosub_css3::system::Css3System;
use gosub_html5::document::document_impl::DocumentImpl;
use gosub_html5::html_compile;
use gosub_html5::parser::Html5Parser;
use gosub_interface::config::ModuleConfiguration;
use gosub_interface::css3::CssSystem as _;
use gosub_interface::document::Document as _;
use gosub_interface::interaction::{InteractionSnapshot, InteractionState};
use gosub_interface::style::Color;
use gosub_shared::node::NodeId;

use crate::common::document::pipeline_doc::{GosubDocumentAdapter, PipelineDocument};

#[derive(Clone, Debug, PartialEq)]
struct Config;

impl ModuleConfiguration for Config {
    type CssSystem = Css3System;
    type Document = DocumentImpl<Self>;
    type HtmlParser = Html5Parser<'static, Self>;
}

const RED: Color = Color::rgba(255, 0, 0, 255);

fn parse(html: &str) -> DocumentImpl<Config> {
    let mut doc = html_compile::<Config>(html);
    doc.add_stylesheet(Css3System::load_default_useragent_stylesheet());
    doc
}

fn by_id(doc: &DocumentImpl<Config>, id: &str) -> NodeId {
    doc.node_by_named_id(id)
        .unwrap_or_else(|| panic!("no element with id {id}"))
}

/// An adapter over `doc` styling with `state`.
fn styled(doc: DocumentImpl<Config>, state: &InteractionState) -> GosubDocumentAdapter<Config> {
    let snapshot = InteractionSnapshot::new::<Config>(state, &doc);
    let adapter = GosubDocumentAdapter::<Config>::new(Arc::new(doc));
    adapter.set_interaction(snapshot);
    adapter
}

fn is_red(adapter: &GosubDocumentAdapter<Config>, id: &str) -> bool {
    adapter.computed_style(by_id(&adapter.doc, id)).inherited.color == RED
}

#[test]
fn hover_matches_the_leaf_and_its_ancestors() {
    let doc = parse(
        r#"<style>div:hover { color: rgb(255, 0, 0) }</style>
        <div id="outer"><div id="inner"><span id="leaf">x</span></div></div><div id="other"></div>"#,
    );
    let state = InteractionState {
        hover_leaf: Some(by_id(&doc, "leaf")),
        ..Default::default()
    };
    let adapter = styled(doc, &state);

    assert!(is_red(&adapter, "outer"));
    assert!(is_red(&adapter, "inner"));
    assert!(!is_red(&adapter, "other"));
}

#[test]
fn hover_follows_an_ancestor_that_moved() {
    let mut doc = parse(
        r#"<style>section:hover { color: rgb(255, 0, 0) }</style>
        <section id="old"><div id="wrap"><span id="leaf">x</span></div></section><section id="new"></section>"#,
    );
    let state = InteractionState {
        hover_leaf: Some(by_id(&doc, "leaf")),
        ..Default::default()
    };
    let (wrap, new) = (by_id(&doc, "wrap"), by_id(&doc, "new"));
    doc.attach_node(wrap, new, None).unwrap();
    let adapter = styled(doc, &state);

    assert!(!is_red(&adapter, "old"), "the old ancestor is no longer hovered");
    assert!(is_red(&adapter, "new"));
}

#[test]
fn focus_focus_visible_and_focus_within() {
    let html = r#"<style>
        #f:focus { color: rgb(255, 0, 0) }
        #v:focus-visible { color: rgb(255, 0, 0) }
        #box:focus-within { color: rgb(255, 0, 0) }
        </style>
        <div id="box"><input id="f"><input id="v"></div><div id="other"></div>"#;

    for (focus, visible) in [("f", false), ("v", true), ("v", false)] {
        let doc = parse(html);
        let state = InteractionState {
            focus: Some(by_id(&doc, focus)),
            focus_visible: visible,
            ..Default::default()
        };
        let adapter = styled(doc, &state);

        assert!(is_red(&adapter, "box"), "{focus} focused: the box holds the focus");
        assert!(!is_red(&adapter, "other"));
        assert_eq!(is_red(&adapter, "f"), focus == "f", "{focus} focused");
        assert_eq!(
            is_red(&adapter, "v"),
            focus == "v" && visible,
            "{focus} focused, ring {visible}"
        );
    }
}

#[test]
fn checked_follows_the_toggle_then_the_attribute() {
    let html = r#"<style>:checked { color: rgb(255, 0, 0) }</style>
        <input type="checkbox" id="on" checked><input type="checkbox" id="off">
        <select id="s"><option id="a">a</option><option id="b">b</option></select>"#;

    let adapter = styled(parse(html), &InteractionState::default());
    assert!(is_red(&adapter, "on"), "the attribute is the default");
    assert!(!is_red(&adapter, "off"));
    assert!(is_red(&adapter, "a"), "the first option is the default choice");
    assert!(!is_red(&adapter, "b"));

    let doc = parse(html);
    let mut state = InteractionState::default();
    state.set_checked(by_id(&doc, "on"), Some(false));
    state.set_checked(by_id(&doc, "off"), Some(true));
    state.set_selected(by_id(&doc, "s"), Some(by_id(&doc, "b")));
    let adapter = styled(doc, &state);
    assert!(!is_red(&adapter, "on"));
    assert!(is_red(&adapter, "off"));
    assert!(!is_red(&adapter, "a"));
    assert!(is_red(&adapter, "b"));
    assert!(
        adapter.is_checked(by_id(&adapter.doc, "off")),
        "the painter's answer agrees"
    );
    assert_eq!(
        adapter.selected_option(by_id(&adapter.doc, "s")),
        Some(by_id(&adapter.doc, "b"))
    );
}

#[test]
fn entries_for_deleted_nodes_are_dropped() {
    let mut doc = parse(
        r#"<style>#box:focus-within, :checked { color: rgb(255, 0, 0) }</style>
        <div id="box"><input id="f"></div>
        <select id="s"><option id="a">a</option><option id="b">b</option></select>"#,
    );
    let (f, s, b) = (by_id(&doc, "f"), by_id(&doc, "s"), by_id(&doc, "b"));
    let mut state = InteractionState {
        focus: Some(f),
        focus_visible: true,
        hover_leaf: Some(f),
        ..Default::default()
    };
    state.set_selected(s, Some(b));
    state.set_edit(
        f,
        Some(gosub_interface::document::ControlEditState::new("typed".into(), 5)),
    );
    doc.delete_node_by_id(f);
    doc.delete_node_by_id(b);
    let adapter = styled(doc, &state);
    let ix = adapter.interaction();

    assert_eq!(ix.focused_node(), None, "focus does not survive its node");
    assert!(!is_red(&adapter, "box"), "nor does focus-within");
    assert!(!ix.is_hovered(by_id(&adapter.doc, "box")));
    assert!(ix.control_edit_state(f).is_none());
    assert!(is_red(&adapter, "a"), "the select is back to its default option");
}
