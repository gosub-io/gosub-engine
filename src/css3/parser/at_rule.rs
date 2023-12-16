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

use crate::css3::node::{Node, NodeType};
use crate::css3::parser::block::BlockParseMode;
use crate::css3::tokenizer::TokenType;
use crate::css3::{Css3, Error};

impl Css3<'_> {

    fn declaration_block_at_rule(&mut self) -> BlockParseMode {
        let mut offset = 1;
        loop {
            let t = self.tokenizer.lookahead(offset);
            offset += 1;

            match t.token_type {
                TokenType::RCurly => {
                    return BlockParseMode::RegularBlock;
                }
                TokenType::LCurly => {
                    return BlockParseMode::RegularBlock;
                }
                TokenType::Eof => {
                    return BlockParseMode::RegularBlock;
                }
                TokenType::AtKeyword(_) => {
                    return BlockParseMode::RegularBlock;
                }
                _ => {
                    // continue
                }
            }
        }
    }

    fn parse_at_rule_prelude(&mut self, name: String) -> Result<Option<Node>, Error> {
        log::trace!("parse_at_rule_prelude");

        self.consume_whitespace_comments();
        let node = match name.to_lowercase().as_str() {
            "container" => Some(self.parse_at_rule_container_prelude()?),
            "font-face" => None,
            "import" => Some(self.parse_at_rule_import_prelude()?),
            "layer" => Some(self.parse_at_rule_layer_prelude()?),
            "media" => Some(self.parse_at_rule_media_prelude()?),
            "nest" => Some(self.parse_at_rule_next_prelude()?),
            "page" => Some(self.parse_at_rule_page_prelude()?),
            "scope" => Some(self.parse_at_rule_scope_prelude()?),
            "starting-style" => None,
            "supports" => Some(self.parse_at_rule_supports_prelude()?),
            // @todo: this should be atRulePrelude scope
            _ => Some(self.parse_selector_list()?),
        };

        self.consume_whitespace_comments();

        let t = self.tokenizer.lookahead(0);
        if !self.tokenizer.eof() && t.token_type != TokenType::Semicolon && t.token_type != TokenType::LCurly {
            return Err(Error::new(
                "Expected semicolon or left curly brace".to_string(),
                t.location.clone(),
            ));
        }

        Ok(node)
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
                let mode = self.declaration_block_at_rule();
                self.parse_block(mode)?
            },
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

        Ok(Node::new(NodeType::AtRule {
            name: name.clone(),
            prelude,
            block,
        }, t.location.clone()))
    }
}
