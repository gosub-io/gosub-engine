use crate::node::{Node, NodeType};
use crate::tokenizer::TokenType;
use crate::{Css3, Error};

impl Css3<'_> {
    fn parse_pseudo_function_selector_list(&mut self) -> Result<Node, Error> {
        log::trace!("parse_pseudo_function_selector_list");
        self.parse_selector_list()
    }

    fn parse_pseudo_function_selector(&mut self) -> Result<Node, Error> {
        log::trace!("parse_pseudo_function_selector");

        self.parse_selector()
    }

    fn parse_pseudo_function_ident_list(&mut self) -> Result<Node, Error> {
        log::trace!("parse_pseudo_function_ident_list");

        let loc = self.tokenizer.current_location();

        let value = self.consume_any_ident()?;

        Ok(Node::new(NodeType::Ident { value }, loc))
    }

    fn parse_pseudo_function_nth(&mut self) -> Result<Node, Error> {
        log::trace!("parse_pseudo_function_nth");

        self.consume_whitespace_comments();

        let loc = self.tokenizer.current_location();

        let mut selector = None;

        let nth = match self.consume_any()?.token_type {
            TokenType::Ident(value) if value == "odd" => Node::new(
                NodeType::AnPlusB {
                    a: "2".into(),
                    b: "1".into(),
                },
                loc.clone(),
            ),
            TokenType::Ident(value) if value == "even" => Node::new(
                NodeType::AnPlusB {
                    a: "2".into(),
                    b: "0".into(),
                },
                loc.clone(),
            ),
            TokenType::Ident(_) => {
                self.tokenizer.reconsume();
                self.parse_anplusb()?
            }
            TokenType::Dimension { .. } => {
                self.tokenizer.reconsume();
                self.parse_anplusb()?
            }
            TokenType::Number(value) => Node::new(NodeType::Number { value }, loc.clone()),
            _ => {
                return Err(Error::new(
                    format!("Unexpected token {:?}", self.tokenizer.lookahead(0)),
                    self.tokenizer.current_location(),
                ));
            }
        };

        self.consume_whitespace_comments();

        let t = self.tokenizer.lookahead(0);
        if let TokenType::Ident(value) = t.token_type {
            self.consume_any()?;

            if value == "of" {
                selector = Some(self.parse_selector_list()?);
            }
        }

        Ok(Node::new(NodeType::Nth { nth, selector }, loc.clone()))
    }

    pub(crate) fn parse_pseudo_function(&mut self, name: &str) -> Result<Node, Error> {
        log::trace!("parse_pseudo_function");
        match name {
            "dir" => self.parse_pseudo_function_ident_list(),
            "has" => self.parse_pseudo_function_selector_list(),
            "lang" => self.parse_pseudo_function_ident_list(),
            "matches" => self.parse_pseudo_function_selector_list(),
            "is" => self.parse_pseudo_function_selector_list(),
            "-moz-any" => self.parse_pseudo_function_selector_list(),
            "-webkit-any" => self.parse_pseudo_function_selector_list(),
            "where" => self.parse_pseudo_function_selector_list(),
            "not" => self.parse_pseudo_function_selector_list(),
            "nth-child" => self.parse_pseudo_function_nth(),
            "nth-last-child" => self.parse_pseudo_function_nth(),
            "nth-last-of-type" => self.parse_pseudo_function_nth(),
            "nth-of-type" => self.parse_pseudo_function_nth(),
            "slotted" => self.parse_pseudo_function_selector(),
            "host" => self.parse_pseudo_function_selector(),
            "host-context" => self.parse_pseudo_function_selector(),
            _ => Err(Error::new(
                format!("Unexpected pseudo function {:?}", name),
                self.tokenizer.current_location(),
            )),
        }
    }
}
