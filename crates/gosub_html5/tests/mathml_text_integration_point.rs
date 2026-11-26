//! Tree construction dispatcher at a MathML text integration point (`mi`, `mo`, `mn`, `ms`,
//! `mtext`): only character tokens and start tags other than `mglyph` / `malignmark` are
//! processed as HTML. End tags, comments and EOF stay in foreign content (GOS-416).
//!
//! Trees are compared in the html5lib-tests line format.
#![allow(clippy::unwrap_used, clippy::expect_used)] // a test that cannot set up must fail loudly

use gosub_css3::system::Css3System;
use gosub_html5::document::builder::DocumentBuilderImpl;
use gosub_html5::document::document_impl::DocumentImpl;
use gosub_html5::parser::Html5Parser;
use gosub_html5::testing::tree_construction::generator::TreeOutputGenerator;
use gosub_interface::config::ModuleConfiguration;
use gosub_interface::document::Document;
use gosub_shared::byte_stream::{ByteStream, Encoding};
use test_case::test_case;

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

// ── end tags ─────────────────────────────────────────────────────────────────

#[test_case("mi")]
#[test_case("mo")]
#[test_case("mn")]
#[test_case("ms")]
#[test_case("mtext")]
fn math_end_tag_closes_math(ip: &str) {
    // Processed as HTML, `</math>` hits the special `<ip>` and is ignored, which left "x"
    // inside it.
    assert_eq!(
        tree(&format!("<math><{ip}></math>x")),
        format!(
            r#"| <html>
|   <head>
|   <body>
|     <math math>
|       <math {ip}>
|     "x""#
        )
    );
}

#[test_case("mi")]
#[test_case("mo")]
#[test_case("mn")]
#[test_case("ms")]
#[test_case("mtext")]
fn html_after_math_end_tag_goes_in_body(ip: &str) {
    assert_eq!(
        tree(&format!("<math><{ip}></math><p>y")),
        format!(
            r#"| <html>
|   <head>
|   <body>
|     <math math>
|       <math {ip}>
|     <p>
|       "y""#
        )
    );
}

#[test_case("mi")]
#[test_case("mo")]
#[test_case("mn")]
#[test_case("ms")]
#[test_case("mtext")]
fn end_tag_of_enclosing_mathml_element(ip: &str) {
    assert_eq!(
        tree(&format!("<math><mrow><{ip}></mrow>x")),
        format!(
            r#"| <html>
|   <head>
|   <body>
|     <math math>
|       <math mrow>
|         <math {ip}>
|       "x""#
        )
    );
}

#[test_case("mi")]
#[test_case("mo")]
#[test_case("mn")]
#[test_case("ms")]
#[test_case("mtext")]
fn own_end_tag(ip: &str) {
    assert_eq!(
        tree(&format!("<math><{ip}></{ip}>x")),
        format!(
            r#"| <html>
|   <head>
|   <body>
|     <math math>
|       <math {ip}>
|       "x""#
        )
    );
}

#[test_case("mi")]
#[test_case("mo")]
#[test_case("mn")]
#[test_case("ms")]
#[test_case("mtext")]
fn p_end_tag_inserts_p(ip: &str) {
    assert_eq!(
        tree(&format!("<math><{ip}></p>x")),
        format!(
            r#"| <html>
|   <head>
|   <body>
|     <math math>
|       <math {ip}>
|         <p>
|         "x""#
        )
    );
}

#[test_case("mi", "div")]
#[test_case("mo", "div")]
#[test_case("mn", "div")]
#[test_case("ms", "div")]
#[test_case("mtext", "div")]
#[test_case("mi", "foo")]
#[test_case("mi", "script")]
#[test_case("mtext", "script")]
fn unrelated_end_tag_is_ignored(ip: &str, end: &str) {
    assert_eq!(
        tree(&format!("<math><{ip}></{end}>x")),
        format!(
            r#"| <html>
|   <head>
|   <body>
|     <math math>
|       <math {ip}>
|         "x""#
        )
    );
}

// ── comments, EOF and character tokens ───────────────────────────────────────

#[test_case("mi")]
#[test_case("mo")]
#[test_case("mn")]
#[test_case("ms")]
#[test_case("mtext")]
fn comment_and_text_go_inside(ip: &str) {
    assert_eq!(
        tree(&format!("<math><{ip}><!--c-->x")),
        format!(
            r#"| <html>
|   <head>
|   <body>
|     <math math>
|       <math {ip}>
|         <!-- c -->
|         "x""#
        )
    );
}

#[test_case("mi")]
#[test_case("mo")]
#[test_case("mn")]
#[test_case("ms")]
#[test_case("mtext")]
fn eof_inside(ip: &str) {
    assert_eq!(
        tree(&format!("<math><{ip}>")),
        format!(
            r#"| <html>
|   <head>
|   <body>
|     <math math>
|       <math {ip}>"#
        )
    );
}

// ── start tags ───────────────────────────────────────────────────────────────

#[test_case("mi", "mglyph")]
#[test_case("mo", "mglyph")]
#[test_case("mn", "mglyph")]
#[test_case("ms", "mglyph")]
#[test_case("mtext", "mglyph")]
#[test_case("mi", "malignmark")]
#[test_case("mtext", "malignmark")]
fn mglyph_and_malignmark_stay_mathml(ip: &str, start: &str) {
    assert_eq!(
        tree(&format!("<math><{ip}><{start}>x")),
        format!(
            r#"| <html>
|   <head>
|   <body>
|     <math math>
|       <math {ip}>
|         <math {start}>
|           "x""#
        )
    );
}

#[test_case("mi")]
#[test_case("mo")]
#[test_case("mn")]
#[test_case("ms")]
#[test_case("mtext")]
fn other_start_tag_is_html(ip: &str) {
    assert_eq!(
        tree(&format!("<math><{ip}><b>x")),
        format!(
            r#"| <html>
|   <head>
|   <body>
|     <math math>
|       <math {ip}>
|         <b>
|           "x""#
        )
    );
}

// ── `</script>` in foreign content ───────────────────────────────────────────

#[test]
fn script_end_tag_only_closes_svg_script() {
    // `</script>` popped whatever the current node was. Now that end tags at an `mi` reach
    // foreign content, that popped the `mi` too.
    assert_eq!(
        tree("<svg><g></script>x"),
        r#"| <html>
|   <head>
|   <body>
|     <svg svg>
|       <svg g>
|         "x""#
    );
    assert_eq!(
        tree("<svg><script></script>x"),
        r#"| <html>
|   <head>
|   <body>
|     <svg svg>
|       <svg script>
|       "x""#
    );
}
