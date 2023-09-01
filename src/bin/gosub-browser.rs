use std::fs::File;

use gosub_engine::html5_parser::input_stream::Confidence;
use gosub_engine::html5_parser::input_stream::{Encoding, InputStream};
use gosub_engine::html5_parser::parser::Html5Parser;

fn main() {
    let file = File::open("../../hello.html").expect("could not open file");

    // We just read the stream from a file. It will use UTF8 as the default encoding.
    let mut stream = InputStream::new();
    stream
        .read_from_file(file, Some(Encoding::UTF8))
        .expect("can't read from file");
    stream.set_confidence(Confidence::Certain);

    // We COULD set the encoding based on external input, like the content-type HTTP header, or
    // maybe a user-setting, or something else that is set by the user-agent)

    // If the encoding confidence is not Confidence::Certain, we should detect the encoding.
    if !stream.is_certain_encoding() {
        stream.detect_encoding()
    }

    let mut parser = Html5Parser::new(&mut stream);
    let _root = parser.parse();
}
