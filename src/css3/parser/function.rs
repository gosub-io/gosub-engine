use crate::css3::node::{Node, NodeType};
use crate::css3::{Css3, Error};
use crate::css3::tokenizer::TokenType;

impl Css3<'_> {

    fn parse_function_arguments(&mut self) -> Result<Vec<Node>, Error> {
        log::trace!("parse_function_arguments");
        self.parse_value_sequence()
    }

    // filter:alpha() can use filter:alpha(opacity=50) as a fallback for IE8
    fn parse_function_arguments_for_alpha(&mut self) -> Result<Vec<Node>, Error> {
        log::trace!("parse_function_arguments_for_alpha");
        self.in_alpha_function = true;
        let args = self.parse_value_sequence()?;
        self.in_alpha_function = false;

        Ok(args)

    }

    pub fn parse_function(&mut self) -> Result<Node, Error> {
        log::trace!("parse_function");

        let loc = self.tokenizer.current_location().clone();

        let name = self.consume_function()?;
        let arguments = if name == "alpha" {
            self.parse_function_arguments_for_alpha()?
        } else {
            self.parse_function_arguments()?
        };

        if ! self.tokenizer.eof() {
            self.consume(TokenType::RParen)?;
        }

        Ok(Node::new(NodeType::Function {
            name,
            arguments,
        }, loc))
    }
}
