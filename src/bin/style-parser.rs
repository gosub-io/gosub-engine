use anyhow::{bail, Result};
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
use gosub_styling::calculator::StyleCalculator;
use gosub_styling::pipeline::Pipeline;
use regex::Regex;
use std::fs;
use url::Url;

struct TextVisitor {
    color: String,
    calculator: StyleCalculator,
}

impl TextVisitor {
    fn new(calculator: StyleCalculator) -> Self {
        Self {
            color: String::from(""),
            calculator,
        }
    }
}

impl Visitor<Node> for TextVisitor {
    fn document_enter(&mut self, _node: &Node, _data: &DocumentData) {}

    fn document_leave(&mut self, _node: &Node, _data: &DocumentData) {}

    fn doctype_enter(&mut self, _node: &Node, _data: &DocTypeData) {}

    fn doctype_leave(&mut self, _node: &Node, _data: &DocTypeData) {}

    fn text_enter(&mut self, _node: &Node, data: &TextData) {
        let re = Regex::new(r"\s{2,}").unwrap();
        let s = re.replace_all(&data.value, " ");

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
        let props = self.calculator.get_css_properties_for_node(node.id);
        if props.is_some() {
            let props = props.unwrap();
            if let Some(col) = props.get_color_value("color") {
                print!("\x1b[38;2;{};{};{}m", col.r, col.g, col.b);
            }
            if let Some(col) = props.get_color_value("background-color") {
                print!("\x1b[48;2;{};{};{}m", col.r, col.g, col.b);
            }
        }

        print!("<{}>", data.name);
    }

    fn element_leave(&mut self, node: &Node, data: &ElementData) {
        let props = self.calculator.get_css_properties_for_node(node.id);
        if props.is_some() {
            let props = props.unwrap();
            if let Some(col) = props.get_color_value("color") {
                print!("\x1b[38;2;{};{};{}m", col.r, col.g, col.b);
            }
            if let Some(col) = props.get_color_value("background-color") {
                print!("\x1b[48;2;{};{};{}m", col.r, col.g, col.b);
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

    // Create stylesheet calculator and load default user agent stylesheets
    let mut calculator = StyleCalculator::new(Document::clone(&doc_handle));
    // calculator.add_stylesheet(load_default_useragent_stylesheet()?);

    // pipeline
    calculator.find_declared_values();
    calculator.find_cascaded_values();
    calculator.find_specified_values();
    // calculator.find_computed_values(1024, 786);     // Do we need more info?
    // calculator.find_used_values(/*layout*/);        // we need to have a layout for calculating these values
    // calculator.find_actual_values();                // Makes sure we use 2px instead of a computed 2.25px

    let pipeline = Pipeline::new();
    let render_tree = pipeline.generate_render_tree(Document::clone(&doc_handle), &calculator);

    let mut visitor = Box::new(TextVisitor::new(calculator)) as Box<dyn Visitor<Node>>;
    visit(&Document::clone(&render_tree), &mut visitor);

    Ok(())
}
