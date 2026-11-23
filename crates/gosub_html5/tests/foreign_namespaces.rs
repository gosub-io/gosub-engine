//! Tree construction asks about HTML elements by name. An SVG or MathML element with the same
//! name is not one of them.
#![allow(clippy::unwrap_used)]

use gosub_css3::system::Css3System;
use gosub_html5::document::builder::DocumentBuilderImpl;
use gosub_html5::document::document_impl::DocumentImpl;
use gosub_html5::parser::Html5Parser;
use gosub_html5::testing::tree_construction::generator::TreeOutputGenerator;
use gosub_interface::config::ModuleConfiguration;
use gosub_interface::document::Document;
use gosub_shared::byte_stream::{ByteStream, Encoding};

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
