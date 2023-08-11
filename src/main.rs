use std::fs::File;
mod html_parser;
use html_parser::input_stream::{InputStream, Encoding};
use crate::html_parser::input_stream::Confidence;
use crate::html_parser::parser::HtmlParser;

fn main() {
    let file = File::open("hello.html").expect("could not open file");

    // We just read the stream from a file. It will use UTF8 as the default encoding.
    let mut stream = InputStream::new();
    stream.read_from_file(file, Some(Encoding::UTF8)).expect("can't read from file");
    stream.set_confidence(Confidence::Certain);

    // We COULD set the encoding based on external input, like the content-type HTTP header, or
    // maybe a user-setting, or something else that is set by the user-agent)

    // If the encoding confidence is not Confidence::Certain, we should detect the encoding.
    if ! stream.is_certain_encoding() {
        stream.detect_encoding()
    }

    stream.reset();
    println!("{}", stream.read_char().unwrap());
    println!("{}", stream.read_char().unwrap());
    println!("{}", stream.read_char().unwrap());
    println!("{}", stream.read_char().unwrap());
    println!("{}", stream.read_char().unwrap());
    println!("{}", stream.read_char().unwrap());
    println!("{}", stream.read_char().unwrap());

    // The unicode bytes is not valid characters anymore
    stream.set_encoding(Encoding::ASCII);

    stream.reset();
    println!("{}", stream.read_char().unwrap());
    println!("{}", stream.read_char().unwrap());
    println!("{}", stream.read_char().unwrap());
    println!("{}", stream.read_char().unwrap());
    println!("{}", stream.read_char().unwrap());
    println!("{}", stream.read_char().unwrap());
    println!("{}", stream.read_char().unwrap());
    println!("{}", stream.read_char().unwrap());

    println!("{}", stream.eof());


    stream.reset();

    let parser = HtmlParser::new(stream);
    parser.parse();
}
