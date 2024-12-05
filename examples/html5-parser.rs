use gosub_css3::system::Css3System;
use gosub_html5::document::builder::DocumentBuilderImpl;
use gosub_html5::document::document_impl::DocumentImpl;
use gosub_html5::document::fragment::DocumentFragmentImpl;
use gosub_html5::parser::Html5Parser;
use gosub_shared::byte_stream::{ByteStream, Encoding};
use gosub_shared::document::DocumentHandle;
use gosub_shared::traits::config::{HasCssSystem, HasDocument, HasHtmlParser};
use gosub_shared::traits::document::DocumentBuilder;
#[derive(Clone, Debug, PartialEq)]
struct Config;

impl HasCssSystem for Config {
    type CssSystem = Css3System;
}
impl HasDocument for Config {
    type Document = DocumentImpl<Self>;
    type DocumentFragment = DocumentFragmentImpl<Self>;
    type DocumentBuilder = DocumentBuilderImpl;
}

impl HasHtmlParser for Config {
    type HtmlParser = Html5Parser<'static, Self>;
}

fn main() {
    // Creates an input stream
    let mut stream = ByteStream::new(Encoding::UTF8, None);
    stream.read_from_str("<p>Hello<b>world</b></p>", Some(Encoding::UTF8));
    stream.close();

    // Initialize a document and feed it together with the stream to the html5 parser
    let doc_handle: DocumentHandle<Config> = DocumentBuilderImpl::new_document(None);

    let _ = Html5Parser::parse_document(&mut stream, doc_handle.clone(), None);

    // document now contains the html5 node tree
    println!("Generated tree: \n\n {}", doc_handle.get());
}
