use crate::node::{Node, NodeType};
use crate::tokenizer::TokenType;
use crate::Css3;
use gosub_shared::errors::CssResult;

impl Css3<'_> {
    fn parse_function_arguments(&mut self) -> CssResult<Vec<Node>> {
        log::trace!("parse_function_arguments");
        self.parse_value_sequence()
    }

    pub fn parse_function(&mut self) -> CssResult<Node> {
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
