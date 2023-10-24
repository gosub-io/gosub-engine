use crate::{css3::new_tokenizer::Tokenizer, html5::input_stream::InputStream};

use super::new_tokenizer::Token;

pub struct CSS3Parser<'stream> {
    tokenizer: Tokenizer<'stream>,
}

impl<'stream> CSS3Parser<'stream> {
    pub fn new(tokenizer: Tokenizer) -> CSS3Parser {
        CSS3Parser { tokenizer }
    }

    pub fn from_input_stream(is: &mut InputStream) -> CSS3Parser {
        CSS3Parser::new(Tokenizer::new(is))
    }

    /// [5.3.1. Parse something according to a CSS grammar](https://www.w3.org/TR/css-syntax-3/#parse-grammar)
    fn parse() {
        todo!()
    }

    /// [5.3.2. Parse A Comma-Separated List According To A CSS Grammar](https://www.w3.org/TR/css-syntax-3/#parse-comma-list)
    fn parse_comma_separated_list() {
        todo!()
    }

    /// [5.4.1. Consume a list of rules](https://www.w3.org/TR/css-syntax-3/#consume-list-of-rules)
    fn consume_list_of_rules(&mut self, is_top_level: bool) {
        // let rules = Vec::new();

        loop {}
    }

    fn current_token(&self) -> &Token {
        self.tokenizer.lookahead(0)
    }

    fn next_token(&self) -> &Token {
        self.tokenizer.lookahead(1)
    }
}
