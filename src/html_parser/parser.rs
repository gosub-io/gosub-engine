use crate::html_parser::tokenizer::Tokenizer;
use crate::html_parser::input_stream::InputStream;
use crate::html_parser::node::Node;

pub struct HtmlParser<'a> {
    tokenizer: Tokenizer<'a>,           // Actual tokenizer
}

impl<'a> HtmlParser<'a> {
    // Creates a new parser object with the given input stream
    pub fn new(stream: &'a mut InputStream) -> Self {
        return HtmlParser {
            tokenizer: Tokenizer::new(stream)
        }
    }

    // Parses the input stream into a Node tree
    pub fn parse(&self) -> Node {
        // Tokenize stuff

        Node::new("root")
    }
}