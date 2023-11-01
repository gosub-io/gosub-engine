use gosub_engine::html5::parser::document::{Document, DocumentBuilder};
use gosub_engine::html5::{
    input_stream::{Encoding, InputStream},
    parser::Html5Parser,
};

fn main() {
    // Creates an input stream
    let mut stream = InputStream::new();
    stream.read_from_str("<p>Hello<b>world</b></p>", Some(Encoding::UTF8));

    // Initialize a document and feed it together with the stream to the html5 parser
    let document = DocumentBuilder::new_document();
    let _ = Html5Parser::parse_document(&mut stream, Document::clone(&document), None);

    // document now contains the html5 node tree
    println!("Generated tree: \n\n {}", document);
}
