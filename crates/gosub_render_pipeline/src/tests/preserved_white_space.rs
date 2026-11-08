//! Preserved white space (`white-space: pre` and `pre-wrap`): segment breaks between inline
//! elements, spaces at the end of one, and tabs (css-text-3 §4.1).

use std::sync::Arc;

use cow_utils::CowUtils as _;
use gosub_css3::system::Css3System;
use gosub_fontmanager::ParleyFontSystem;
use gosub_html5::document::document_impl::DocumentImpl;
use gosub_html5::html_compile;
use gosub_html5::parser::Html5Parser;
use gosub_interface::config::ModuleConfiguration;
use gosub_interface::css3::CssSystem as _;
use gosub_interface::document::Document as _;
use gosub_interface::font_system::FontSystem;
use gosub_shared::node::NodeId;
use parking_lot::Mutex;

use crate::common::document::pipeline_doc::GosubDocumentAdapter;
use crate::common::geo::{Dimension, Rect};
use crate::common::media::MediaStore;
use crate::layouter::taffy::TaffyLayouter;
use crate::layouter::{CanLayout as _, ElementContext, LayoutTree};
use crate::rendertree_builder::tree::RenderTree;

#[derive(Clone, Debug, PartialEq)]
struct Config;

impl ModuleConfiguration for Config {
    type CssSystem = Css3System;
    type Document = DocumentImpl<Self>;
    type HtmlParser = Html5Parser<'static, Self>;
}

/// `html` laid out in an 800x600 viewport, and the node of each `id` asked for.
fn lay_out<const N: usize>(html: &str, ids: [&str; N]) -> (LayoutTree, [NodeId; N]) {
    let mut doc = html_compile::<Config>(html);
    doc.add_stylesheet(Css3System::load_default_useragent_stylesheet());
    let adapter = GosubDocumentAdapter::<Config>::new(Arc::new(doc));
    let nodes = ids.map(|id| {
        fn find(a: &GosubDocumentAdapter<Config>, node: NodeId, id: &str) -> Option<NodeId> {
            if a.doc.attribute(node, "id") == Some(id) {
                return Some(node);
            }
            a.doc.children(node).iter().find_map(|&child| find(a, child, id))
        }
        find(&adapter, adapter.doc.root(), id).unwrap_or_else(|| panic!("no element with id {id}"))
    });
    let mut render_tree = RenderTree::new(Arc::new(adapter));
    render_tree.parse().expect("render tree");
    let font_system: Arc<Mutex<dyn FontSystem>> = Arc::new(Mutex::new(ParleyFontSystem::new()));
    let mut layouter = TaffyLayouter::with_font_system_and_media_store(font_system, Arc::new(MediaStore::new()));
    let tree = layouter.layout(render_tree, Some(Dimension::new(800.0, 600.0)), 1.0);
    (tree, nodes)
}

/// Whether `node` is `ancestor` or inside it.
fn within(tree: &LayoutTree, mut node: NodeId, ancestor: NodeId) -> bool {
    loop {
        if node == ancestor {
            return true;
        }
        match tree.render_tree.doc.parent(node) {
            Some(parent) => node = parent,
            None => return false,
        }
    }
}

/// Every piece of text inside `node` with its border box, in reading order.
fn text_pieces(tree: &LayoutTree, node: NodeId) -> Vec<(String, Rect)> {
    let mut pieces: Vec<(String, Rect)> = tree
        .arena
        .values()
        .filter_map(|el| match &el.context {
            ElementContext::Text(text) if within(tree, el.dom_node_id, node) => {
                Some((text.text.clone(), el.box_model.border_box))
            }
            _ => None,
        })
        .collect();
    pieces.sort_by(|(_, a), (_, b)| a.y.total_cmp(&b.y).then(a.x.total_cmp(&b.x)));
    pieces
}

/// The pieces of text inside `node`, grouped into the lines they sit on.
fn lines(tree: &LayoutTree, node: NodeId) -> Vec<Vec<(String, Rect)>> {
    let mut lines: Vec<Vec<(String, Rect)>> = Vec::new();
    for piece in text_pieces(tree, node) {
        match lines.last_mut() {
            Some(line) if (line[0].1.y - piece.1.y).abs() < 1.0 => line.push(piece),
            _ => lines.push(vec![piece]),
        }
    }
    lines
}

/// The words of each line, as a line of text.
fn line_texts(tree: &LayoutTree, node: NodeId) -> Vec<String> {
    lines(tree, node)
        .iter()
        .map(|line| {
            line.iter()
                .map(|(text, _)| text.cow_replace('\u{a0}', " ").into_owned())
                .collect::<String>()
                .trim()
                .to_string()
        })
        .collect()
}

#[test]
fn a_preserved_newline_between_inline_elements_breaks_the_line() {
    let (tree, [mixed, plain]) = lay_out(
        "<body style=\"margin: 0\">\
         <pre id=\"mixed\">one <b>bold</b>\ntwo <i>it</i>\nthree</pre>\
         <pre id=\"plain\">plain one\nplain two</pre></body>",
        ["mixed", "plain"],
    );
    assert_eq!(line_texts(&tree, mixed), ["one bold", "two it", "three"]);
    let lines = lines(&tree, mixed);
    let start = lines[0][0].1.x;
    for line in &lines {
        assert!(
            (line[0].1.x - start).abs() < 0.5,
            "each line starts at the left edge: {lines:?}"
        );
    }
    let height = lines[1][0].1.y - lines[0][0].1.y;
    assert!(height > 0.0);
    assert!(
        (lines[2][0].1.y - lines[1][0].1.y - height).abs() < 0.5,
        "the lines are evenly spaced: {lines:?}"
    );
    assert_eq!(text_pieces(&tree, plain).len(), 1, "a pre of only text stays one run");
}

#[test]
fn a_preserved_newline_inside_an_inline_element_breaks_the_line() {
    let (tree, [pre]) = lay_out(
        "<body style=\"margin: 0\"><pre id=\"pre\">one <b>bold\nnext</b> more\nlast</pre></body>",
        ["pre"],
    );
    assert_eq!(line_texts(&tree, pre), ["one bold", "next more", "last"]);
}

#[test]
fn two_preserved_newlines_leave_a_blank_line() {
    let (tree, [pre]) = lay_out(
        "<body style=\"margin: 0\"><pre id=\"pre\" style=\"line-height: 20px\">one <b>a</b>\ntwo <b>b</b>\n\nfour</pre></body>",
        ["pre"],
    );
    let lines = lines(&tree, pre);
    assert_eq!(line_texts(&tree, pre), ["one a", "two b", "four"]);
    let pitch = lines[1][0].1.y - lines[0][0].1.y;
    assert!(
        (lines[2][0].1.y - lines[1][0].1.y - 2.0 * pitch).abs() < 0.5,
        "an empty line sits between two and four: {lines:?}"
    );
}

#[test]
fn a_preserved_newline_breaks_the_line_under_pre_wrap() {
    let (tree, [pre]) = lay_out(
        "<body style=\"margin: 0\"><pre id=\"pre\" style=\"white-space: pre-wrap\">one <b>bold</b>\ntwo <i>it</i>\nthree</pre></body>",
        ["pre"],
    );
    assert_eq!(line_texts(&tree, pre), ["one bold", "two it", "three"]);
}

#[test]
fn pre_wrap_text_among_inline_elements_still_wraps() {
    let (tree, [pre]) = lay_out(
        "<body style=\"margin: 0\"><pre id=\"pre\" style=\"white-space: pre-wrap; width: 120px\">\
         <b>a</b> one two three four five six seven\nend</pre></body>",
        ["pre"],
    );
    let pieces = text_pieces(&tree, pre);
    let long = pieces
        .iter()
        .find(|(text, _)| text.contains("seven"))
        .expect("the long piece");
    let end = pieces.iter().find(|(text, _)| text == "end").expect("the last line");
    assert!(long.1.width <= 120.0, "the long piece wraps inside the pre: {pieces:?}");
    assert!(
        long.1.height > end.1.height * 1.5,
        "onto more than one line: {pieces:?}"
    );
    assert!(
        end.1.y >= long.1.y + long.1.height - 0.5,
        "and the line after it starts below it"
    );
}

#[test]
fn a_newline_among_inline_elements_still_collapses_under_normal() {
    let (tree, [div]) = lay_out(
        "<body style=\"margin: 0\"><div id=\"div\">one <b>bold</b>\ntwo <i>it</i>\nthree</div></body>",
        ["div"],
    );
    assert_eq!(line_texts(&tree, div), ["one bold two it three"]);
}
