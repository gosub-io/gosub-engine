use crate::node::{Node, NodeType};
use crate::Css3;
use gosub_shared::errors::CssResult;

impl Css3<'_> {
    pub fn parse_at_rule_page_prelude(&mut self) -> CssResult<Node> {
        log::trace!("parse_at_rule_page_prelude");

        let loc = self.tokenizer.current_location();

        let mut selectors = vec![];

        while !self.tokenizer.eof() {
            let selector = self.parse_selector()?;
            selectors.push(selector);

            self.consume_whitespace_comments();

            let t = self.consume_any()?;
            if !t.is_comma() {
                self.tokenizer.reconsume();
                break;
            }
        }

        Ok(Node::new(NodeType::SelectorList { selectors }, loc))
    }
}
