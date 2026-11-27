//! Tree construction asks about HTML elements by name. An SVG or MathML element with the same
//! name is not one of them.
#![allow(clippy::unwrap_used)]

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

/// The document tree `html` parses to, in the html5lib test format.
fn tree(html: &str) -> String {
    let mut doc = DocumentBuilderImpl::new_document::<Config>(None);
    let mut stream = ByteStream::from_str(html, Encoding::UTF8);
    Html5Parser::<Config>::parse_document(&mut stream, &mut doc, None).unwrap();
    let root = doc.root();
    TreeOutputGenerator::<Config>::new(doc).generate_from(root).join("\n")
}

/// The tree of `html` parsed as a fragment with a `name` element in `namespace` as context, the
/// context sitting under an HTML `body`.
fn fragment(namespace: &str, name: &str, html: &str) -> String {
    parse_fragment(namespace, name, html).0
}

/// [`fragment`], with the parse errors.
fn parse_fragment(namespace: &str, name: &str, html: &str) -> (String, Vec<String>) {
    let mut doc = DocumentBuilderImpl::new_document::<Config>(None);
    let body = doc.create_element("body", Some(HTML_NAMESPACE), Attributes::new(), Location::default());
    let root = doc.root();
    doc.attach(body, root, None).unwrap();
    let context = doc.create_element(name, Some(namespace), Attributes::new(), Location::default());
    doc.attach(context, body, None).unwrap();

    let mut stream = ByteStream::from_str(html, Encoding::UTF8);
    let errors =
        Html5Parser::<Config>::parse_fragment(&mut stream, &mut doc, context, None, Location::default()).unwrap();

    // The parser appends its own `html` element to the document root; the fragment is below it.
    let html_id = *doc.children(doc.root()).last().unwrap();
    let tree = TreeOutputGenerator::<Config>::new(doc)
        .generate_from(html_id)
        .join("\n");
    (tree, errors.into_iter().map(|error| error.message).collect())
}

/// The parse error messages `html` produces.
fn errors(html: &str) -> Vec<String> {
    let mut doc = DocumentBuilderImpl::new_document::<Config>(None);
    let mut stream = ByteStream::from_str(html, Encoding::UTF8);
    let errors = Html5Parser::<Config>::parse_document(&mut stream, &mut doc, None).unwrap();
    errors.into_iter().map(|error| error.message).collect()
}

fn lines(lines: &[&str]) -> String {
    lines.join("\n")
}

/// `</template>` closes an HTML template only. With just an SVG `template` open the end tag is
/// ignored, where it used to empty the stack and leave what follows outside `<html>`.
#[test]
fn an_svg_template_is_not_closed_by_a_template_end_tag() {
    assert_eq!(
        tree("<!DOCTYPE html><svg><template><foreignObject><div></template><p>after"),
        lines(&[
            "| <!DOCTYPE html>",
            "| <html>",
            "|   <head>",
            "|   <body>",
            "|     <svg svg>",
            "|       <svg template>",
            "|         <svg foreignObject>",
            "|           <div>",
            "|             <p>",
            "|               \"after\"",
        ])
    );
}

/// A `<frameset>` that replaces the body pops everything above the document's `html`, not up to
/// whatever element is named `html`.
#[test]
fn a_frameset_unwinds_past_an_svg_html_element() {
    assert_eq!(
        tree("<svg><html><foreignObject><frameset><frame>"),
        lines(&["| <html>", "|   <head>", "|   <frameset>", "|     <frame>"])
    );
}

/// Resetting the insertion mode looks at HTML elements only, so an SVG `frameset`, `select` or
/// context element does not pick the mode.
#[test]
fn foreign_elements_do_not_choose_the_insertion_mode() {
    // Closing the template resets the mode; an SVG frameset used to make it "in frameset",
    // which dropped the div.
    assert_eq!(
        tree("<svg><frameset><foreignObject><template></template><div>x"),
        lines(&[
            "| <html>",
            "|   <head>",
            "|   <body>",
            "|     <svg svg>",
            "|       <svg frameset>",
            "|         <svg foreignObject>",
            "|           <template>",
            "|             content",
            "|           <div>",
            "|             \"x\"",
        ])
    );
    // An SVG select used to give "in select in table", whose recovery emptied the stack.
    assert_eq!(
        tree("<table><svg><select><foreignObject><template></template><tr>"),
        lines(&[
            "| <html>",
            "|   <head>",
            "|   <body>",
            "|     <svg svg>",
            "|       <svg select>",
            "|         <svg foreignObject>",
            "|           <template>",
            "|             content",
            "|     <table>",
            "|       <tbody>",
            "|         <tr>",
        ])
    );
    // A fragment with an SVG select as context starts "in body", so a breakout div is kept.
    assert_eq!(
        fragment(SVG_NAMESPACE, "select", "<div>x</div>"),
        lines(&["| <div>", "|   \"x\""])
    );
}

/// An end tag in body closes an HTML element of that name. Walking down from the current node
/// it stops at the first special element instead, and an SVG `desc` or `title` is one, so the
/// end tag is ignored and the text stays in the span.
#[test]
fn an_end_tag_does_not_close_an_svg_element_of_that_name() {
    for name in ["desc", "title"] {
        assert_eq!(
            tree(&format!("<!DOCTYPE html><svg><{name}><span></{name}>after")),
            lines(&[
                "| <!DOCTYPE html>",
                "| <html>",
                "|   <head>",
                "|   <body>",
                "|     <svg svg>",
                &format!("|       <svg {name}>"),
                "|         <span>",
                "|           \"after\"",
            ]),
            "{name}"
        );
    }
}

/// Leaving the body with an element open that is not one of the listed HTML ones is a parse
/// error, and a foreign element is never one of them: at EOF, at `</body>` and at `</html>`.
#[test]
fn an_open_foreign_element_at_the_end_of_the_body_is_an_error() {
    const OPEN: &str = "open element not allowed at end of body";
    for html in [
        "<!DOCTYPE html><svg><g>",
        "<!DOCTYPE html><math><mi>x",
        // The end tags reach the HTML rules with the svg and g still open.
        "<!DOCTYPE html><svg><g></body>",
        "<!DOCTYPE html><svg><g></html>",
    ] {
        let errors = errors(html);
        assert!(errors.iter().any(|error| error == OPEN), "{html}: {errors:?}");
    }
    // The listed HTML elements may stay open.
    let errors = errors("<!DOCTYPE html><p>x");
    assert!(!errors.iter().any(|error| error == OPEN), "{errors:?}");
}

/// Only an HTML `template` context starts the fragment "in template". An SVG one used to push
/// that mode anyway, so EOF went to the template rules and skipped the end-of-body check.
#[test]
fn an_svg_template_context_is_not_a_template() {
    let (tree, errors) = parse_fragment(SVG_NAMESPACE, "template", "<div>x");
    assert_eq!(tree, lines(&["| <div>", "|   \"x\""]));
    assert!(
        errors
            .iter()
            .any(|error| error == "open element not allowed at end of body"),
        "{errors:?}"
    );

    // An HTML template context still is one: its contents are parsed "in template".
    let (_, errors) = parse_fragment(HTML_NAMESPACE, "template", "<div>x");
    assert!(
        !errors
            .iter()
            .any(|error| error == "open element not allowed at end of body"),
        "{errors:?}"
    );
}

/// Closing a cell pops to the HTML `td` or `th`. An SVG `td` or `th` left as the current node
/// is not the cell, so that is a parse error, like any other element there. The tree does not
/// change: the token that closed the cell clears the stack back to the row either way.
#[test]
fn closing_a_cell_does_not_stop_at_an_svg_td_or_th() {
    const NOT_A_CELL: &str = "current node should be td or th";
    for html in [
        // The end tag closes the cell.
        "<!DOCTYPE html><table><tr><td><svg><td></table>x",
        "<!DOCTYPE html><table><tr><td><svg><th></tr>x",
        "<!DOCTYPE html><table><tr><td><math><mi><svg><td></table>x",
        // A start tag at an integration point closes the cell.
        "<!DOCTYPE html><table><tr><td><svg><td><foreignObject><tr>y",
        // Any other foreign element was already reported.
        "<!DOCTYPE html><table><tr><td><svg><g></table>x",
    ] {
        let errors = errors(html);
        assert!(errors.iter().any(|error| error == NOT_A_CELL), "{html}: {errors:?}");
    }
    let errors = errors("<!DOCTYPE html><table><tr><td>x</table>");
    assert!(!errors.iter().any(|error| error == NOT_A_CELL), "{errors:?}");

    assert_eq!(
        tree("<!DOCTYPE html><table><tr><td><svg><td></table>x"),
        lines(&[
            "| <!DOCTYPE html>",
            "| <html>",
            "|   <head>",
            "|   <body>",
            "|     <table>",
            "|       <tbody>",
            "|         <tr>",
            "|           <td>",
            "|             <svg svg>",
            "|               <svg td>",
            "|     \"x\"",
        ])
    );
    assert_eq!(
        tree("<!DOCTYPE html><table><tr><td><svg><td><foreignObject><tr>y"),
        lines(&[
            "| <!DOCTYPE html>",
            "| <html>",
            "|   <head>",
            "|   <body>",
            "|     \"y\"",
            "|     <table>",
            "|       <tbody>",
            "|         <tr>",
            "|           <td>",
            "|             <svg svg>",
            "|               <svg td>",
            "|                 <svg foreignObject>",
            "|         <tr>",
        ])
    );
}

/// `</h1>`..`</h6>` pop to the HTML heading. Heading start tags break out of foreign content, so
/// no foreign heading is ever open; this pins the tree for the other caller of the same pop.
#[test]
fn a_heading_end_tag_pops_to_the_html_heading() {
    assert_eq!(
        tree("<!DOCTYPE html><h1><svg><h2></h1>x"),
        lines(&[
            "| <!DOCTYPE html>",
            "| <html>",
            "|   <head>",
            "|   <body>",
            "|     <h1>",
            "|       <svg svg>",
            "|     <h2>",
            "|     \"x\"",
        ])
    );
}
