use crate::css3::{Css3, Error};
use crate::css3::node::{Node, NodeType};

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
        let value = self.consume_any_ident()?;

        Ok(Node::new(NodeType::Ident{ value }))
    }

    fn parse_pseudo_function_nth(&mut self) -> Result<Node, Error> {
        log::trace!("parse_pseudo_function_nth");
        todo!("parse_pseudo_function_nth")
        // self.parse_nth()
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
                self.tokenizer.current_location.clone(),
            )),
        }
    }

}