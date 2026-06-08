use crate::node::{Node, NodeType};
use crate::tokenizer::TokenType;
use crate::Css3;
use gosub_shared::errors::{CssError, CssResult};

const MAX_FUNCTION_DEPTH: usize = 256;

impl Css3<'_> {
    fn parse_function_arguments(&mut self) -> CssResult<Vec<Node>> {
        log::trace!("parse_function_arguments");
        self.parse_value_sequence()
    }

    pub fn parse_function(&mut self) -> CssResult<Node> {
        log::trace!("parse_function");

        if self.function_depth >= MAX_FUNCTION_DEPTH {
            return Err(CssError::with_location(
                "function nesting too deep",
                self.tokenizer.current_location(),
            ));
        }

        let loc = self.tokenizer.current_location();

        let name = self.consume_function()?;
        self.function_depth += 1;
        let arguments = if name == "alpha" {
            self.allow_values_in_argument_list.push(true);
            let args = self.parse_function_arguments();
            self.allow_values_in_argument_list.pop();
            self.function_depth -= 1;
            args?
        } else {
            let args = self.parse_function_arguments();
            self.function_depth -= 1;
            args?
        };

        if !self.tokenizer.eof() {
            self.consume(TokenType::RParen)?;
        }

        Ok(Node::new(NodeType::Function { name, arguments }, loc))
    }
}
