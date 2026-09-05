//! Declarative shadow DOM tree construction: `<template shadowrootmode>`.
//!
//! Trees are compared in the html5lib-tests line format, which the tree-construction harness
//! also produces, with the shadow tree printed under its host ahead of the light children.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use gosub_css3::system::Css3System;
use gosub_html5::document::builder::DocumentBuilderImpl;
use gosub_html5::document::document_impl::DocumentImpl;
use gosub_html5::parser::Html5Parser;
use gosub_html5::testing::tree_construction::generator::TreeOutputGenerator;
use gosub_interface::config::{HasDocument, ModuleConfiguration};
use gosub_interface::document::Document;
use gosub_interface::node::{NodeType, ShadowRootMode, SlotAssignmentMode};
use gosub_shared::byte_stream::{ByteStream, Encoding, Location};
use gosub_shared::node::NodeId;

#[derive(Clone, Debug, PartialEq)]
struct Config;

impl ModuleConfiguration for Config {
    type CssSystem = Css3System;
    type Document = DocumentImpl<Self>;
    type HtmlParser = Html5Parser<'static, Self>;
}

fn parse(html: &str) -> DocumentImpl<Config> {
    let mut stream = ByteStream::from_str(html, Encoding::UTF8);
    let mut doc = DocumentBuilderImpl::new_document::<Config>(None);
    let _ = Html5Parser::<Config>::parse_document(&mut stream, &mut doc, None);
    doc
}

/// Parses `html` and returns the tree in the html5lib-tests line format.
fn tree(html: &str) -> String {
    let doc = parse(html);
    let root = doc.root();
    TreeOutputGenerator::<Config>::new(doc).generate_from(root).join("\n")
}

/// Depth-first search for the first element with the given tag name.
fn find<C: HasDocument>(doc: &C::Document, tag: &str) -> Option<NodeId> {
    fn walk<C: HasDocument>(doc: &C::Document, id: NodeId, tag: &str) -> Option<NodeId> {
        if doc.tag_name(id) == Some(tag) {
            return Some(id);
        }
        doc.children(id)
            .to_vec()
            .into_iter()
            .find_map(|c| walk::<C>(doc, c, tag))
    }
    walk::<C>(doc, doc.root(), tag)
}

// ── the shadow tree is built, and lives off the host ─────────────────────────

#[test]
fn a_declarative_template_becomes_a_shadow_root() {
    // The template element itself is gone: only the shadow root it declared remains, and it
    // is not among the host's children.
    assert_eq!(
        tree("<div><template shadowrootmode=open><span>shadow</span></template></div>"),
        r#"| <html>
|   <head>
|   <body>
|     <div>
|       #shadow-root (open)
|         <span>
|           "shadow""#
    );
}

#[test]
fn light_children_stay_siblings_of_the_shadow_root() {
    // The shadow root is printed first because it is not a child at all - the light children
    // below it are the host's entire `children` list.
    assert_eq!(
        tree("<div><template shadowrootmode=open><slot></slot></template><p>light</p></div>"),
        r#"| <html>
|   <head>
|   <body>
|     <div>
|       #shadow-root (open)
|         <slot>
|       <p>
|         "light""#
    );
}

#[test]
fn the_host_points_at_the_root_and_the_root_back_at_the_host() {
    let doc = parse("<div><template shadowrootmode=closed></template></div>");
    let host = find::<Config>(&doc, "div").unwrap();
    let root = doc.shadow_root(host).unwrap();

    assert_eq!(doc.node_type(root), NodeType::ShadowRootNode);
    assert_eq!(doc.shadow_host(root), Some(host));
    assert!(
        !doc.children(host).contains(&root),
        "shadow root must stay out of children"
    );
    // A shadow root has no parent, so no ancestor walk can climb out of the shadow tree.
    assert_eq!(doc.parent(root), None);
}

#[test]
fn the_declarative_attributes_are_recorded() {
    let doc = parse(
        "<div><template shadowrootmode=CLOSED shadowrootdelegatesfocus shadowrootclonable \
         shadowrootserializable></template></div>",
    );
    let host = find::<Config>(&doc, "div").unwrap();
    let init = doc.shadow_root_init(doc.shadow_root(host).unwrap()).unwrap();

    // The mode attribute is ASCII case-insensitive.
    assert_eq!(init.mode, ShadowRootMode::Closed);
    assert!(init.delegates_focus);
    assert!(init.clonable);
    assert!(init.serializable);
    // Declarative shadow roots are always named; there is no attribute for manual assignment.
    assert_eq!(init.slot_assignment, SlotAssignmentMode::Named);
}

#[test]
fn nested_shadow_roots_each_hang_off_their_own_host() {
    assert_eq!(
        tree(
            "<div><template shadowrootmode=open><section><template shadowrootmode=closed>\
             <b>inner</b></template></section></template></div>"
        ),
        r#"| <html>
|   <head>
|   <body>
|     <div>
|       #shadow-root (open)
|         <section>
|           #shadow-root (closed)
|             <b>
|               "inner""#
    );
}

// ── everything that must fall back to an inert template ──────────────────────

#[test]
fn a_template_without_the_attribute_is_untouched() {
    assert_eq!(
        tree("<div><template><span>x</span></template></div>"),
        r#"| <html>
|   <head>
|   <body>
|     <div>
|       <template>
|         content
|           <span>
|             "x""#
    );
}

#[test]
fn an_unrecognised_mode_value_is_the_none_state() {
    // `shadowrootmode` is an enumerated attribute: anything but open/closed leaves it in the
    // "none" state, and the template stays inert.
    let doc = parse("<div><template shadowrootmode=sideways></template></div>");
    assert_eq!(doc.shadow_root(find::<Config>(&doc, "div").unwrap()), None);
    assert!(find::<Config>(&doc, "template").is_some());
}

#[test]
fn an_ineligible_host_keeps_an_inert_template() {
    // <b> is not a valid shadow host name, so "attach a shadow root" throws and the parser
    // recovers by inserting the template as usual.
    let doc = parse("<b><template shadowrootmode=open><i>x</i></template></b>");
    assert_eq!(doc.shadow_root(find::<Config>(&doc, "b").unwrap()), None);
    assert!(find::<Config>(&doc, "template").is_some());
}

#[test]
fn a_custom_element_is_an_eligible_host() {
    let doc = parse("<my-widget><template shadowrootmode=open><i>x</i></template></my-widget>");
    assert!(doc.shadow_root(find::<Config>(&doc, "my-widget").unwrap()).is_some());
}

#[test]
fn a_second_shadow_root_on_one_host_stays_a_template() {
    // The host already has a shadow root, so the second template is inserted normally - into
    // the light DOM, where the first one no longer is.
    let doc = parse(
        "<div><template shadowrootmode=open><i>first</i></template>\
         <template shadowrootmode=open><i>second</i></template></div>",
    );
    let host = find::<Config>(&doc, "div").unwrap();
    let root = doc.shadow_root(host).unwrap();

    assert_eq!(doc.children(root).len(), 1);
    assert_eq!(doc.children(host).len(), 1);
    assert_eq!(doc.tag_name(doc.children(host)[0]), Some("template"));
}

#[test]
fn a_document_level_template_hosts_nothing() {
    // With no open element to host it, the template is handled in "in head" and stays inert.
    // Two independent rules reject it: the intended parent is <head>, which is not a valid
    // shadow host name, and the topmost-element guard excludes <html> once <head> is closed.
    let doc = parse("<template shadowrootmode=open></template>");
    let html = doc.children(doc.root())[0];

    assert_eq!(doc.shadow_root(html), None);
    for child in doc.children(html) {
        assert_eq!(doc.shadow_root(*child), None);
    }
    assert!(find::<Config>(&doc, "template").is_some());
}

#[test]
fn fragment_parsing_never_builds_shadow_roots() {
    // innerHTML must not create shadow roots; that needs setHTMLUnsafe().
    let mut doc = DocumentBuilderImpl::new_document::<Config>(None);
    let host = doc.create_element("div", None, std::collections::HashMap::new(), Location::default());
    doc.attach(host, doc.root(), None);

    let mut stream = ByteStream::from_str("<template shadowrootmode=open><i>x</i></template>", Encoding::UTF8);
    let _ = Html5Parser::<Config>::parse_fragment(&mut stream, &mut doc, host, None, Location::default());

    assert_eq!(doc.shadow_root(host), None);
}

// ── serialisation round-trips ────────────────────────────────────────────────

#[test]
fn a_shadow_root_is_written_back_out_as_its_template() {
    let doc = parse("<div><template shadowrootmode=open><span>s</span></template><p>l</p></div>");
    let host = find::<Config>(&doc, "div").unwrap();

    assert_eq!(
        doc.write_from_node(host),
        r#"<div><template shadowrootmode="open"><span>s</span></template><p>l</p></div>"#
    );
}

#[test]
fn the_boolean_declarative_attributes_survive_a_round_trip() {
    let doc = parse("<div><template shadowrootmode=closed shadowrootdelegatesfocus></template></div>");
    let host = find::<Config>(&doc, "div").unwrap();

    assert_eq!(
        doc.write_from_node(host),
        r#"<div><template shadowrootmode="closed" shadowrootdelegatesfocus=""></template></div>"#
    );
}
