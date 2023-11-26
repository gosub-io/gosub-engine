use crate::css3::node::{Node, NodeType};
use crate::css3::{Css3, Error};
use crate::css3::tokenizer::TokenType;

impl Css3<'_> {

    fn parse_function_arguments(&mut self) -> Result<Vec<Node>, Error> {
        log::trace!("parse_function_arguments");
        self.parse_value_sequence()
    }

    pub fn parse_function(&mut self) -> Result<Node, Error> {
        log::trace!("parse_function");
        let name = self.consume_function()?;
        let arguments = self.parse_function_arguments()?;

        if ! self.tokenizer.eof() {
            self.consume(TokenType::RParen)?;
        }

        Ok(Node::new(NodeType::Function {
            name,
            arguments,
        }))
    }
}
