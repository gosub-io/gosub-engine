use gosub_css3::system::Css3System;
use gosub_html5::document::builder::DocumentBuilderImpl;
use gosub_html5::document::document_impl::DocumentImpl;
use gosub_html5::parser::Html5Parser;
use gosub_interface::config::ModuleConfiguration;

use gosub_shared::byte_stream::{ByteStream, Encoding};
#[derive(Clone, Debug, PartialEq)]
struct Config;

impl ModuleConfiguration for Config {
    type CssSystem = Css3System;
    type Document = DocumentImpl<Self>;
    type HtmlParser = Html5Parser<'static, Self>;
}

fn main() {
    // Creates an input stream
    let mut stream = ByteStream::from_str("<p>Hello<b>world</b></p>", Encoding::UTF8);

    // Initialize a document and feed it together with the stream to the html5 parser
    let mut doc = DocumentBuilderImpl::new_document::<Config>(None);

    let _ = Html5Parser::<Config>::parse_document(&mut stream, &mut doc, None);

    // document now contains the html5 node tree
    println!("Generated tree: \n\n {doc}");
}
