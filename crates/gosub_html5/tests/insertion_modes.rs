//! Tree construction cases from the insertion modes, each checked against the spec's steps.
#![allow(clippy::unwrap_used)]

use gosub_css3::system::Css3System;
use gosub_html5::document::builder::DocumentBuilderImpl;
use gosub_html5::document::document_impl::DocumentImpl;
use gosub_html5::parser::Html5Parser;
use gosub_html5::testing::tree_construction::generator::TreeOutputGenerator;
use gosub_interface::config::ModuleConfiguration;
use gosub_interface::document::{Document, DocumentType};
use gosub_interface::node::QuirksMode;
use gosub_shared::byte_stream::{ByteStream, Encoding};

#[derive(Clone, Debug, PartialEq)]
struct Config;

impl ModuleConfiguration for Config {
    type CssSystem = Css3System;
    type Document = DocumentImpl<Self>;
    type HtmlParser = Html5Parser<'static, Self>;
}

/// What one parse produced.
struct Parsed {
    tree: String,
    errors: Vec<String>,
    quirks: QuirksMode,
}

fn parse_as(html: &str, doctype: DocumentType) -> Parsed {
    let mut doc = DocumentBuilderImpl::new_document::<Config>(None);
    doc.set_doctype(doctype);
    let mut stream = ByteStream::from_str(html, Encoding::UTF8);
    let errors = Html5Parser::<Config>::parse_document(&mut stream, &mut doc, None).unwrap();
    let quirks = doc.quirks_mode();
    let root = doc.root();
    Parsed {
        tree: TreeOutputGenerator::<Config>::new(doc).generate_from(root).join("\n"),
        errors: errors.into_iter().map(|error| error.message).collect(),
        quirks,
    }
}

fn parse(html: &str) -> Parsed {
    parse_as(html, DocumentType::HTML)
}

/// An iframe srcdoc document without a DOCTYPE is neither an error nor in quirks mode
/// ("initial" insertion mode, anything else).
#[test]
fn a_srcdoc_document_without_a_doctype_stays_in_no_quirks_mode() {
    let parsed = parse_as("<p>x", DocumentType::IframeSrcDoc);
    assert_eq!(parsed.quirks, QuirksMode::NoQuirks);
    assert!(parsed.errors.is_empty(), "{:?}", parsed.errors);
    assert_eq!(
        parsed.tree,
        ["| <html>", "|   <head>", "|   <body>", "|     <p>", "|       \"x\""].join("\n")
    );

    // An ordinary document still is.
    let parsed = parse("<p>x");
    assert_eq!(parsed.quirks, QuirksMode::Quirks);
}

/// Whitespace in a `thead` stays there, as it does in `tbody` and `tfoot` ("in table": a
/// character token while the current node is one of table, tbody, template, tfoot, thead, tr).
#[test]
fn whitespace_in_a_thead_stays_in_the_thead() {
    let parsed = parse("<!DOCTYPE html><table><thead> \n<tr><th>H</th></tr></thead></table>");
    assert_eq!(
        parsed.tree,
        [
            "| <!DOCTYPE html>",
            "| <html>",
            "|   <head>",
            "|   <body>",
            "|     <table>",
            "|       <thead>",
            "|         \" ",
            "\"",
            "|         <tr>",
            "|           <th>",
            "|             \"H\"",
        ]
        .join("\n")
    );
    assert!(parsed.errors.is_empty(), "{:?}", parsed.errors);
}

/// `</table>` and `<table>` in "in table" act only on a table in table scope, which ends at a
/// `template`: a table outside the template is not closed from inside it.
#[test]
fn a_table_tag_inside_a_template_does_not_reach_the_table_outside() {
    // `</table>`: ignored, so the div goes into the template contents.
    let parsed = parse("<!DOCTYPE html><table><template><tbody></tbody></table><div>x</div>");
    assert_eq!(
        parsed.tree,
        [
            "| <!DOCTYPE html>",
            "| <html>",
            "|   <head>",
            "|   <body>",
            "|     <table>",
            "|       <template>",
            "|         content",
            "|           <tbody>",
            "|           <div>",
            "|             \"x\"",
        ]
        .join("\n")
    );

    // `<table>`: ignored too, so the row still lands in the template.
    let parsed = parse("<!DOCTYPE html><table><template><tbody><table><tr>");
    assert_eq!(
        parsed.tree,
        [
            "| <!DOCTYPE html>",
            "| <html>",
            "|   <head>",
            "|   <body>",
            "|     <table>",
            "|       <template>",
            "|         content",
            "|           <tbody>",
            "|             <tr>",
        ]
        .join("\n")
    );
}
