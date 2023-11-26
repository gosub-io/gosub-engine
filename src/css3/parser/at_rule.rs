mod container;
mod font_face;
mod import;
mod layer;
mod media;
mod next;
mod page;
mod scope;
mod starting_style;
mod supports;

use crate::css3::node::Node;
use crate::css3::parser::block::BlockParseMode;
use crate::css3::tokenizer::TokenType;
use crate::css3::{Css3, Error};

impl Css3<'_> {
    fn parse_at_rule_prelude(&mut self, name: String) -> Result<Option<Node>, Error> {
        log::trace!("parse_at_rule_prelude");

        self.consume_whitespace_comments();
        let node = match name.to_lowercase().as_str() {
            "container" => self.parse_at_rule_container()?,
            "font-face" => self.parse_at_rule_font_face()?,
            "import" => self.parse_at_rule_import()?,
            "layer" => self.parse_at_rule_layer()?,
            "media" => self.parse_at_rule_media()?,
            "nest" => self.parse_at_rule_next()?,
            "page" => self.parse_at_rule_page()?,
            "scope" => self.parse_at_rule_scope()?,
            "starting-style" => self.parse_at_rule_starting_style()?,
            "supports" => self.parse_at_rule_supports()?,
            _ => self.parse_selector_list()?,
        };

        let t = self.tokenizer.current();

        if t.token_type == TokenType::Semicolon || t.token_type == TokenType::LCurly {
            // Seems there is no prelude
            return Ok(None);
        }

        Ok(Some(node))
    }

    fn parse_at_rule_block(
        &mut self,
        name: String,
        is_declaration: bool,
    ) -> Result<Option<Node>, Error> {
        log::trace!("parse_at_rule_block");

        let t = self.tokenizer.consume();
        if t.token_type != TokenType::LCurly {
            // Seems there is no block
            return Ok(None);
        }

        // @Todo: maybe this is the other way around. Need to verify this
        let mut mode = BlockParseMode::RegularBlock;
        if is_declaration {
            mode = BlockParseMode::StyleBlock;
        }

        // parse block. They may or may not have nested rules depending on the is_declaration and block type
        let node = match name.to_lowercase().as_str() {
            "container" => self.parse_block(mode)?,
            "font-face" => self.parse_block(BlockParseMode::StyleBlock)?,
            "layer" => self.parse_block(BlockParseMode::RegularBlock)?,
            "media" => self.parse_block(mode)?,
            "nest" => self.parse_block(BlockParseMode::StyleBlock)?,
            "page" => self.parse_block(BlockParseMode::StyleBlock)?,
            "scope" => self.parse_block(mode)?,
            "starting-style" => self.parse_block(mode)?,
            "supports" => self.parse_block(mode)?,
            _ => {
                todo!("We have to figure out if we are a isDeclaration or not atrule)");
                // self.parse_block()?
            }
        };

        self.consume(TokenType::RCurly)?;

        Ok(Some(node))
    }

    pub fn parse_at_rule(&mut self, is_declaration: bool) -> Result<Node, Error> {
        log::trace!("parse_at_rule");

        let name;

        let t = self.consume_any()?;
        if let TokenType::AtKeyword(keyword) = t.token_type {
            name = keyword;
        } else {
            return Err(Error::new("Expected at keyword".to_string(), t.location));
        }
        self.consume_whitespace_comments();

        let prelude = self.parse_at_rule_prelude(name.clone())?;
        self.consume_whitespace_comments();

        let block = self.parse_at_rule_block(name.clone(), is_declaration)?;
        self.consume_whitespace_comments();

        Ok(Node::new_at_rule(name.clone(), prelude, block))
    }
}
