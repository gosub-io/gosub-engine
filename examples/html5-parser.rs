use gosub_html5::parser::document::{Document, DocumentBuilder};
use gosub_html5::parser::Html5Parser;
use gosub_shared::bytes::{CharIterator, Encoding};

fn main() {
    // Creates an input stream
    let mut chars = CharIterator::new();
    chars.read_from_str("<p>Hello<b>world</b></p>", Some(Encoding::UTF8));

    // Initialize a document and feed it together with the stream to the html5 parser
    let document = DocumentBuilder::new_document(None);
    let _ = Html5Parser::parse_document(&mut chars, Document::clone(&document), None);

    // document now contains the html5 node tree
    println!("Generated tree: \n\n {}", document);
}
