use crate::node::{Node, NodeType};
use crate::tokenizer::TokenType;
use crate::{Css3, Error};

impl Css3<'_> {
    fn parse_function_arguments(&mut self) -> Result<Vec<Node>, Error> {
        log::trace!("parse_function_arguments");
        self.parse_value_sequence()
    }

    pub fn parse_function(&mut self) -> Result<Node, Error> {
        log::trace!("parse_function");

        let loc = self.tokenizer.current_location();

        let name = self.consume_function()?;
        let arguments = if name == "alpha" {
            self.allow_values_in_argument_list.push(true);
            let args = self.parse_function_arguments()?;
            self.allow_values_in_argument_list.pop();
            args
        } else {
            self.parse_function_arguments()?
        };

        if !self.tokenizer.eof() {
            self.consume(TokenType::RParen)?;
        }

        Ok(Node::new(NodeType::Function { name, arguments }, loc))
    }
}
