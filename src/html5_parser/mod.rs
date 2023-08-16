pub mod input_stream;

mod consume_char_refs;
mod emitter;
mod node;
mod token;
mod token_named_characters;
mod token_replacements;
mod token_states;
mod tokenizer;

use input_stream::InputStream;
use node::Node;
use tokenizer::Tokenizer;

pub struct Html5Parser<'a> {
    tokenizer: Tokenizer<'a>,
}

impl<'a> Html5Parser<'a> {
    // Creates a new parser object with the given input stream
    pub fn new(stream: &'a mut InputStream) -> Self {
        Html5Parser {
            tokenizer: Tokenizer::new(stream),
        }
    }

    // Parses the input stream into a Node tree
    pub fn parse(&mut self) -> Node {
        // Tokenize stuff

        for _ in 1..=20 {
            let t = self.tokenizer.next_token();
            println!("{}", t.to_string());
        }

        let mut n = Node::new("root");
        n.add_child(Node::new("child"));
        return n;
    }
}
