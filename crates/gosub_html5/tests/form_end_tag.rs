//! `</form>` outside a template closes the form element pointer's own element, and only when
//! that element is in scope. Another form being in scope is not enough. The expected trees
//! follow the spec, Blink and html5ever; Gecko checks by tag name here.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

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

/// Parses `html` and returns the tree in the html5lib-tests line format.
fn tree(html: &str) -> String {
    let mut stream = ByteStream::from_str(html, Encoding::UTF8);
    let mut doc = DocumentBuilderImpl::new_document::<Config>(None);
    let _ = Html5Parser::<Config>::parse_document(&mut stream, &mut doc, None);
    let root = doc.root();
    TreeOutputGenerator::<Config>::new(doc).generate_from(root).join("\n")
}

/// Leaves form `x` open with the pointer on form `p`, which is no longer on the stack: the
/// first `</form>` is ignored behind the marquee and clears the pointer, then the in-table
/// `<form>` sets the pointer and is popped at once.
const SETUP: &str = "<!DOCTYPE html><form id=x><marquee></form></marquee><table><form id=p></table>";

const SETUP_TREE: &str = r#"| <!DOCTYPE html>
| <html>
|   <head>
|   <body>
|     <form>
|       id="x"
|       <marquee>
|       <table>
|         <form>
|           id="p""#;

#[test]
fn another_form_in_scope_does_not_close_an_open_paragraph() {
    assert_eq!(
        tree(&format!("{SETUP}<p>a</form>b")),
        format!(
            r#"{SETUP_TREE}
|       <p>
|         "ab""#
        )
    );
}

#[test]
fn another_form_in_scope_does_not_close_an_open_list_item() {
    assert_eq!(
        tree(&format!("{SETUP}<ul><li>a</form>b")),
        format!(
            r#"{SETUP_TREE}
|       <ul>
|         <li>
|           "ab""#
        )
    );
}

#[test]
fn another_form_in_scope_is_not_closed() {
    assert_eq!(
        tree(&format!("{SETUP}a</form>b</form>c")),
        format!(
            r#"{SETUP_TREE}
|       "abc""#
        )
    );
}

#[test]
fn the_pointers_form_in_scope_is_closed() {
    assert_eq!(
        tree("<form><p>a</form>b"),
        r#"| <html>
|   <head>
|   <body>
|     <form>
|       <p>
|         "a"
|     "b""#
    );
}

#[test]
fn the_pointers_form_behind_a_scope_boundary_is_not_closed() {
    assert_eq!(
        tree("<form><marquee></form>a"),
        r#"| <html>
|   <head>
|   <body>
|     <form>
|       <marquee>
|         "a""#
    );
}

#[test]
fn a_form_inside_a_template_is_still_closed_by_name() {
    assert_eq!(
        tree("<template><form><p>a</form>b</template>"),
        r#"| <html>
|   <head>
|     <template>
|       content
|         <form>
|           <p>
|             "a"
|         "b"
|   <body>"#
    );
}
