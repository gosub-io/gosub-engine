use gosub_engine::html5_parser::input_stream::InputStream;
use gosub_engine::html5_parser::parser::Html5Parser;

fn main() {
    let mut input_stream = InputStream::new();
    input_stream.read_from_str("<html><p id=\"hello\" class=\"one two\">hello</p></html>", None);

    let mut parser = Html5Parser::new(&mut input_stream);
    let (document, _parse_error) = parser.parse();
}
