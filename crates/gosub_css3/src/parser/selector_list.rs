use crate::node::{Node, NodeType};
use crate::Css3;
use gosub_shared::errors::CssResult;

impl Css3<'_> {
    pub fn parse_selector_list(&mut self) -> CssResult<Node> {
        log::trace!("parse_selector_list");

        let loc = self.tokenizer.current_location();

        let mut selectors = vec![];

        while !self.tokenizer.eof() {
            let selector = self.parse_selector()?;
            selectors.push(selector);

            let t = self.consume_any()?;
            if !t.is_comma() {
                self.tokenizer.reconsume();
                break;
            }
        }

        Ok(Node::new(NodeType::SelectorList { selectors }, loc))
    }
}
