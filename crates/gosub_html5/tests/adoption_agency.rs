//! The adoption agency algorithm (`</b>` and friends). The expected trees are Firefox's.
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

// step 4.3: the search stops at the last marker

#[test]
fn a_cell_hides_formatting_elements_from_before_it() {
    // The cell pushed a marker, so `</b>` does not see the `b` from before it and is ignored.
    // The `b` stays in the list and is reopened around the text after the table.
    assert_eq!(
        tree("<p><b></p><table><tr><td></b>x</td></tr></table>y"),
        r#"| <html>
|   <head>
|   <body>
|     <p>
|       <b>
|     <table>
|       <tbody>
|         <tr>
|           <td>
|             "x"
|     <b>
|       "y""#
    );
}

#[test]
fn a_header_cell_hides_formatting_elements_from_before_it() {
    assert_eq!(
        tree("<p><b></p><table><tr><th></b>x</th></tr></table>y"),
        r#"| <html>
|   <head>
|   <body>
|     <p>
|       <b>
|     <table>
|       <tbody>
|         <tr>
|           <th>
|             "x"
|     <b>
|       "y""#
    );
}

#[test]
fn a_caption_hides_formatting_elements_from_before_it() {
    assert_eq!(
        tree("<p><b></p><table><caption></b>x</caption></table>y"),
        r#"| <html>
|   <head>
|   <body>
|     <p>
|       <b>
|     <table>
|       <caption>
|         "x"
|     <b>
|       "y""#
    );
}

#[test]
fn a_cell_in_a_cell_hides_formatting_elements_from_both_tables() {
    // `</i>` stops at the inner cell's marker; the `i` from before the outer table survives
    // and is reopened around "w".
    assert_eq!(
        tree("<p><i></p><table><tr><td><b>x<table><tr><td></i></b>y</td></tr></table>z</td></tr></table>w"),
        r#"| <html>
|   <head>
|   <body>
|     <p>
|       <i>
|     <table>
|       <tbody>
|         <tr>
|           <td>
|             <b>
|               "x"
|               <table>
|                 <tbody>
|                   <tr>
|                     <td>
|                       "y"
|               "z"
|     <i>
|       "w""#
    );
}

// step 4.5: the formatting element itself must be in scope

#[test]
fn a_table_between_hides_the_formatting_element() {
    // The fourth `b` was dropped from the list (more than three alike), so the formatting element
    // for `</b>` is the one with the id, below the table. The open `b` above the table has the
    // same name, but only the element itself counts: it is out of scope and `</b>` is ignored.
    assert_eq!(
        tree("<b id=1><table><b><b><b><b></b></b></b><span></b>x"),
        r#"| <html>
|   <head>
|   <body>
|     <b>
|       id="1"
|       <b>
|         <b>
|           <b>
|             <b>
|         <span>
|           "x"
|       <table>"#
    );
}

#[test]
fn a_foreign_object_between_hides_the_formatting_element() {
    assert_eq!(
        tree("<b id=1><svg><foreignObject><b><b><b><b></b></b></b><span></b>x"),
        r#"| <html>
|   <head>
|   <body>
|     <b>
|       id="1"
|       <svg svg>
|         <svg foreignObject>
|           <b>
|             <b>
|               <b>
|                 <b>
|             <span>
|               "x""#
    );
}

#[test]
fn a_mathml_text_integration_point_between_hides_the_formatting_element() {
    assert_eq!(
        tree("<b id=1><math><mi><b><b><b><b></b></b></b><span></b>x"),
        r#"| <html>
|   <head>
|   <body>
|     <b>
|       id="1"
|       <math math>
|         <math mi>
|           <b>
|             <b>
|               <b>
|                 <b>
|             <span>
|               "x""#
    );
}

// unchanged

#[test]
fn an_applet_reopens_formatting_elements_before_its_marker() {
    // Unchanged by the marker check: `applet`, `object` and `marquee` reopen the `b` before
    // pushing their marker, so `</b>` finds that one.
    assert_eq!(
        tree("<p><b></p><applet></b>x</applet>y"),
        r#"| <html>
|   <head>
|   <body>
|     <p>
|       <b>
|     <b>
|       <applet>
|         "x"
|       "y""#
    );
}

#[test]
fn an_object_reopens_formatting_elements_before_its_marker() {
    assert_eq!(
        tree("<p><b></p><object></b>x</object>y"),
        r#"| <html>
|   <head>
|   <body>
|     <p>
|       <b>
|     <b>
|       <object>
|         "x"
|       "y""#
    );
}

#[test]
fn a_marquee_reopens_formatting_elements_before_its_marker() {
    assert_eq!(
        tree("<p><b></p><marquee></b>x</marquee>y"),
        r#"| <html>
|   <head>
|   <body>
|     <p>
|       <b>
|     <b>
|       <marquee>
|         "x"
|       "y""#
    );
}

#[test]
fn a_template_ignores_a_stray_end_tag() {
    assert_eq!(
        tree("<p><b></p><template></b>x</template>y"),
        r#"| <html>
|   <head>
|   <body>
|     <p>
|       <b>
|     <template>
|       content
|         "x"
|     <b>
|       "y""#
    );
}

#[test]
fn a_formatting_element_after_the_marker_is_closed_as_usual() {
    assert_eq!(
        tree("<b>1<table><tr><td><b>2</b>3</td></tr></table>4"),
        r#"| <html>
|   <head>
|   <body>
|     <b>
|       "1"
|       <table>
|         <tbody>
|           <tr>
|             <td>
|               <b>
|                 "2"
|               "3"
|       "4""#
    );
}

#[test]
fn a_same_named_element_without_a_boundary_between_is_in_scope() {
    assert_eq!(
        tree("<b id=1><b><b><b><b></b></b></b><span></b>x"),
        r#"| <html>
|   <head>
|   <body>
|     <b>
|       id="1"
|       <b>
|         <b>
|           <b>
|             <b>
|         <span>
|     "x""#
    );
}

#[test]
fn a_misnested_formatting_end_tag_is_adopted() {
    assert_eq!(
        tree("<b>1<p>2</b>3"),
        r#"| <html>
|   <head>
|   <body>
|     <b>
|       "1"
|     <p>
|       <b>
|         "2"
|       "3""#
    );
}

#[test]
fn a_furthest_block_is_reparented_under_a_clone() {
    assert_eq!(
        tree("<a><b><div></a>x"),
        r#"| <html>
|   <head>
|   <body>
|     <a>
|       <b>
|     <b>
|       <div>
|         <a>
|         "x""#
    );
}
