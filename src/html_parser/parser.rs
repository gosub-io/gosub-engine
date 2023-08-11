use crate::html_parser::tokenizer::Tokenizer;
use crate::html_parser::input_stream::InputStream;
use crate::html_parser::node::Node;

pub struct HtmlParser {
    tokenizer: Tokenizer,           // Actual tokenizer
}

impl HtmlParser {
    // Creates a new parser object with the given input stream
    pub fn new(stream: InputStream) -> Self {
        return HtmlParser {
            tokenizer: Tokenizer::new(stream)
        }
    }

    // Parses the input stream into a Node tree
    pub fn parse(&self) -> Node {
        // Tokenize stuff

        Node::new(String::from("root"))
    }
}