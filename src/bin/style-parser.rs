use std::fs;

use anyhow::{bail, Result};
use url::Url;

use gosub_html5::node::data::comment::CommentData;
use gosub_html5::node::data::doctype::DocTypeData;
use gosub_html5::node::data::document::DocumentData;
use gosub_html5::node::data::element::ElementData;
use gosub_html5::node::data::text::TextData;
use gosub_html5::node::Node;
use gosub_html5::parser::document::DocumentBuilder;
use gosub_html5::parser::document::{visit, Document};
use gosub_html5::parser::Html5Parser;
use gosub_html5::visit::Visitor;
use gosub_shared::bytes::{CharIterator, Confidence, Encoding};
use gosub_styling::css_values::CssValue;
use gosub_styling::render_tree::{generate_render_tree, RenderTree};

struct TextVisitor {
    color: String,
    css_nodetree: RenderTree,
}

impl TextVisitor {
    fn new(css_node_tree: RenderTree) -> Self {
        Self {
            color: String::from(""),
            css_nodetree: css_node_tree,
        }
    }
}

impl Visitor<Node> for TextVisitor {
    fn document_enter(&mut self, _node: &Node, _data: &DocumentData) {}

    fn document_leave(&mut self, _node: &Node, _data: &DocumentData) {}

    fn doctype_enter(&mut self, _node: &Node, _data: &DocTypeData) {}

    fn doctype_leave(&mut self, _node: &Node, _data: &DocTypeData) {}

    fn text_enter(&mut self, _node: &Node, data: &TextData) {
        // let re = Regex::new(r"\s{2,}").unwrap();
        // let s = re.replace_all(&data.value, " ");
        let s = &data.value;

        if !self.color.is_empty() {
            print!("\x1b[{}m", self.color)
        }

        if !s.is_empty() {
            print!("{}", s)
        }

        if !self.color.is_empty() {
            print!("\x1b[0m")
        }
    }

    fn text_leave(&mut self, _node: &Node, _data: &TextData) {}

    fn comment_enter(&mut self, _node: &Node, _data: &CommentData) {}

    fn comment_leave(&mut self, _node: &Node, _data: &CommentData) {}

    fn element_enter(&mut self, node: &Node, data: &ElementData) {
        if let Some(mut prop) = self.css_nodetree.get_property(node.id, "color") {
            if let CssValue::Color(col) = prop.compute_value() {
                self.color = format!("\x1b[38;2;{};{};{}m", col.r, col.g, col.b)
            }
        }

        if let Some(mut prop) = self.css_nodetree.get_property(node.id, "background-color") {
            if let CssValue::Color(col) = prop.compute_value() {
                print!("\x1b[48;2;{};{};{}m", col.r, col.g, col.b)
            }
        }

        print!("<{}>", data.name);
    }

    fn element_leave(&mut self, node: &Node, data: &ElementData) {
        if let Some(mut prop) = self.css_nodetree.get_property(node.id, "color") {
            if let CssValue::Color(col) = prop.compute_value() {
                self.color = format!("\x1b[38;2;{};{};{}m", col.r, col.g, col.b)
            }
        }

        if let Some(mut prop) = self.css_nodetree.get_property(node.id, "background-color") {
            if let CssValue::Color(col) = prop.compute_value() {
                print!("\x1b[48;2;{};{};{}m", col.r, col.g, col.b)
            }
        }

        print!("</{}>", data.name);
        print!("\x1b[39;49m"); // default terminal color reset
    }
}

fn main() -> Result<()> {
    let matches = clap::Command::new("Gosub Style parser")
        .version("0.1.0")
        .arg(
            clap::Arg::new("url")
                .help("The url or file to parse")
                .required(true)
                .index(1),
        )
        .get_matches();

    let str_url: String = matches.get_one::<String>("url").expect("url").to_string();
    let url = Url::parse(&str_url)?;

    let html = if url.scheme() == "http" || url.scheme() == "https" {
        // Fetch the html from the url
        let response = ureq::get(url.as_ref()).call()?;
        if response.status() != 200 {
            bail!(format!(
                "Could not get url. Status code {}",
                response.status()
            ));
        }
        response.into_string()?
    } else if url.scheme() == "file" {
        fs::read_to_string(str_url.trim_start_matches("file://"))?
    } else {
        bail!("Unsupported url scheme: {}", url.scheme());
    };

    let mut chars = CharIterator::new();
    chars.read_from_str(&html, Some(Encoding::UTF8));
    chars.set_confidence(Confidence::Certain);

    let doc_handle = DocumentBuilder::new_document(Some(url));
    let _parse_errors =
        Html5Parser::parse_document(&mut chars, Document::clone(&doc_handle), None)?;

    let render_tree = generate_render_tree(Document::clone(&doc_handle))?;

    let mut visitor = Box::new(TextVisitor::new(render_tree)) as Box<dyn Visitor<Node>>;
    visit(&Document::clone(&doc_handle), &mut visitor);

    Ok(())
}
