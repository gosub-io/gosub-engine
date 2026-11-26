//! Fragment parsing sets the form element pointer to the nearest `form` at or above the
//! context element (HTML fragment parsing algorithm, step 11). With the pointer set, a `<form>`
//! start tag in the fragment is ignored, which is what these tests look at.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use gosub_css3::system::Css3System;
use gosub_html5::document::builder::DocumentBuilderImpl;
use gosub_html5::document::document_impl::DocumentImpl;
use gosub_html5::node::{HTML_NAMESPACE, SVG_NAMESPACE};
use gosub_html5::parser::Html5Parser;
use gosub_html5::testing::tree_construction::generator::TreeOutputGenerator;
use gosub_interface::config::ModuleConfiguration;
use gosub_interface::document::Document;
use gosub_shared::attributes::Attributes;
use gosub_shared::byte_stream::{ByteStream, Encoding, Location};

#[derive(Clone, Debug, PartialEq)]
struct Config;

impl ModuleConfiguration for Config {
    type CssSystem = Css3System;
    type Document = DocumentImpl<Self>;
    type HtmlParser = Html5Parser<'static, Self>;
}

/// Builds the chain `ancestors` below the document root, each element the child of the one
/// before, parses `html` with the last one as context, and returns the fragment's tree.
fn fragment(ancestors: &[(&str, &str)], html: &str) -> String {
    let mut doc = DocumentBuilderImpl::new_document::<Config>(None);
    let mut parent = doc.root();
    for (name, namespace) in ancestors {
        let element = doc.create_element(name, Some(namespace), Attributes::new(), Location::default());
        doc.attach(element, parent, None).unwrap();
        parent = element;
    }
    let context = parent;

    let mut stream = ByteStream::from_str(html, Encoding::UTF8);
    Html5Parser::<Config>::parse_fragment(&mut stream, &mut doc, context, None, Location::default()).unwrap();

    // The parser appends its own `html` element to the document root; the fragment is below it.
    let html_id = *doc.children(doc.root()).last().unwrap();
    TreeOutputGenerator::<Config>::new(doc)
        .generate_from(html_id)
        .join("\n")
}

const FORM: (&str, &str) = ("form", HTML_NAMESPACE);
const DIV: (&str, &str) = ("div", HTML_NAMESPACE);

#[test]
fn a_form_start_tag_inside_a_form_ancestor_is_ignored() {
    let tree = fragment(&[FORM, DIV], "<form id=inner><input>");
    assert!(!tree.contains("<form>"), "{tree}");
    assert!(tree.contains("<input>"), "{tree}");
}

#[test]
fn the_form_ancestor_is_found_further_up() {
    let tree = fragment(
        &[FORM, ("section", HTML_NAMESPACE), ("p", HTML_NAMESPACE), DIV],
        "<form><input>",
    );
    assert!(!tree.contains("<form>"), "{tree}");
}

#[test]
fn a_form_context_element_sets_the_pointer() {
    let tree = fragment(&[FORM], "<form><input>");
    assert!(!tree.contains("<form>"), "{tree}");
}

#[test]
fn without_a_form_ancestor_the_form_is_inserted() {
    let tree = fragment(&[DIV], "<form><input>");
    assert!(tree.contains("<form>"), "{tree}");
}

#[test]
fn a_form_in_another_namespace_does_not_count() {
    let tree = fragment(&[("form", SVG_NAMESPACE), DIV], "<form><input>");
    assert!(tree.contains("<form>"), "{tree}");
}

#[test]
fn the_search_stops_at_template_contents() {
    let mut doc = DocumentBuilderImpl::new_document::<Config>(None);
    let root = doc.root();
    let form = doc.create_element("form", Some(HTML_NAMESPACE), Attributes::new(), Location::default());
    doc.attach(form, root, None).unwrap();
    let template = doc.create_element("template", Some(HTML_NAMESPACE), Attributes::new(), Location::default());
    doc.attach(template, form, None).unwrap();
    // How the parser stores template contents: as the template's own children.
    doc.set_template_contents(template, template);
    let div = doc.create_element("div", Some(HTML_NAMESPACE), Attributes::new(), Location::default());
    doc.attach(div, template, None).unwrap();

    let mut stream = ByteStream::from_str("<form><input>", Encoding::UTF8);
    Html5Parser::<Config>::parse_fragment(&mut stream, &mut doc, div, None, Location::default()).unwrap();

    let html_id = *doc.children(doc.root()).last().unwrap();
    let tree = TreeOutputGenerator::<Config>::new(doc)
        .generate_from(html_id)
        .join("\n");
    assert!(tree.contains("<form>"), "{tree}");
}

#[test]
fn in_table_a_form_inside_a_form_ancestor_is_ignored() {
    let tree = fragment(&[FORM, ("table", HTML_NAMESPACE)], "<form><tr><td>x");
    assert!(!tree.contains("<form>"), "{tree}");
    assert!(tree.contains("<td>"), "{tree}");
}

#[test]
fn an_end_tag_clears_the_pointer_from_the_ancestor() {
    // `</form>` takes the pointer even though that form is not on the stack, so a later form
    // start tag is inserted.
    let tree = fragment(&[FORM, DIV], "</form><form id=later>");
    assert!(tree.contains("<form>"), "{tree}");
}
