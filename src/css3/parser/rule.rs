use crate::css3::node::{Node, NodeType};
use crate::css3::parser::block::BlockParseMode;
use crate::css3::tokenizer::TokenType;
use crate::css3::{Css3, Error};

impl Css3<'_> {
    pub fn parse_rule(&mut self) -> Result<Node, Error> {
        log::trace!("parse_rule");
        let loc = self.tokenizer.current_location().clone();

        let prelude = self.parse_selector_list()?;

        self.consume(TokenType::LCurly)?;
        self.consume_whitespace_comments();

        let block = self.parse_block(BlockParseMode::StyleBlock)?;

        self.consume(TokenType::RCurly)?;

        Ok(Node::new(NodeType::Rule{ prelude: Some(prelude), block: Some(block) }, loc))
    }
}
