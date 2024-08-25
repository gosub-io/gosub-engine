use gosub_html5::parser::document::{Document, DocumentBuilder};
use gosub_html5::parser::Html5Parser;
use gosub_shared::byte_stream::{ByteStream, Encoding};

fn main() {
    // Creates an input stream
    let mut stream = ByteStream::new(Encoding::UTF8, None);
    stream.read_from_str("<p>Hello<b>world</b></p>", Some(Encoding::UTF8));
    stream.close();

    // Initialize a document and feed it together with the stream to the html5 parser
    let document = DocumentBuilder::new_document(None);
    let _ = Html5Parser::parse_document(&mut stream, Document::clone(&document), None);

    // document now contains the html5 node tree
    println!("Generated tree: \n\n {}", document);
}
