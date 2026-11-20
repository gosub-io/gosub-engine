//! The class list is derived from the `class` attribute, so the two never disagree (GOS-411).

use gosub_shared::attributes::Attributes;

use gosub_css3::system::Css3System;
use gosub_html5::document::builder::DocumentBuilderImpl;
use gosub_html5::document::document_impl::DocumentImpl;
use gosub_html5::node::HTML_NAMESPACE;
use gosub_html5::parser::Html5Parser;
use gosub_interface::config::ModuleConfiguration;
use gosub_interface::document::Document;
use gosub_shared::byte_stream::{ByteStream, Encoding, Location};
use gosub_shared::node::NodeId;

#[derive(Clone, Debug, PartialEq)]
struct Config;

impl ModuleConfiguration for Config {
    type CssSystem = Css3System;
    type Document = DocumentImpl<Self>;
    type HtmlParser = Html5Parser<'static, Self>;
}

fn element(class: Option<&str>) -> (DocumentImpl<Config>, NodeId) {
    let mut doc = DocumentBuilderImpl::new_document::<Config>(None);
    let mut attributes = Attributes::new();
    if let Some(class) = class {
        attributes.insert("class".into(), class.into());
    }
    let id = doc.create_element("p", Some(HTML_NAMESPACE), attributes, Location::default());
    doc.attach(id, doc.root(), None);
    (doc, id)
}

#[test]
fn a_created_element_has_the_classes_of_its_attribute() {
    let (doc, p) = element(Some("x y"));
    assert!(doc.has_class(p, "x"));
    assert!(doc.has_class(p, "y"));
    assert!(!doc.has_class(p, "x y"));
}

#[test]
fn removing_the_class_attribute_removes_every_class() {
    let (mut doc, p) = element(Some("x y"));
    doc.remove_attribute(p, "class");
    assert!(!doc.has_class(p, "x"));
    assert!(!doc.has_class(p, "y"));
}

#[test]
fn setting_the_class_attribute_replaces_the_classes() {
    let (mut doc, p) = element(Some("x"));
    doc.set_attribute(p, "class", "y z");
    assert!(!doc.has_class(p, "x"));
    assert!(doc.has_class(p, "y") && doc.has_class(p, "z"));
}

#[test]
fn add_class_shows_in_the_attribute_and_the_markup() {
    let (mut doc, p) = element(None);
    doc.add_class(p, "z");
    assert_eq!(doc.attribute(p, "class"), Some("z"));
    assert!(doc.has_class(p, "z"));
    assert_eq!(doc.write_from_node(p), "<p class=\"z\"></p>");

    doc.add_class(p, "w");
    assert_eq!(doc.write_from_node(p), "<p class=\"z w\"></p>");
}

#[test]
fn add_class_after_removing_the_attribute_starts_from_empty() {
    let (mut doc, p) = element(Some("x"));
    doc.remove_attribute(p, "class");
    doc.add_class(p, "y");
    assert_eq!(doc.attribute(p, "class"), Some("y"));
    assert!(!doc.has_class(p, "x"));
}

#[test]
fn a_parsed_class_splits_on_ascii_whitespace_only() {
    // U+00A0 is not ASCII whitespace: `a\u{a0}b` is one class, as the selector index and the
    // ancestor filter already read it.
    let mut stream = ByteStream::from_str("<p class=\"a\u{a0}b\tc\"></p>", Encoding::UTF8);
    let mut doc = DocumentBuilderImpl::new_document::<Config>(None);
    assert!(Html5Parser::<Config>::parse_document(&mut stream, &mut doc, None).is_ok());
    // document -> html -> head -> body -> p
    let p = NodeId::from(4usize);
    assert_eq!(doc.tag_name(p), Some("p"));
    assert!(doc.has_class(p, "a\u{a0}b"));
    assert!(doc.has_class(p, "c"));
    assert!(!doc.has_class(p, "a"));
}
