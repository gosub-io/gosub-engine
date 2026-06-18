use crate::node::{Node, NodeType};
use crate::tokenizer::TokenType;
use crate::Css3;
use gosub_shared::errors::{CssError, CssResult};

#[derive(Debug, PartialEq)]
pub enum BlockParseMode {
    StyleBlock,
    RegularBlock,
}

impl Css3<'_> {
    fn parse_consume_rule(&mut self) -> CssResult<Option<Node>> {
        log::trace!("parse_consume_rule");
        self.parse_rule()
    }

    /// Disambiguates, inside a style block, between a nested style rule and a declaration.
    ///
    /// Per CSS Nesting, a construct is a nested rule when its prelude is followed by a `{ ... }`
    /// block, and a declaration when it terminates at a `;` or at the block's closing `}`. We
    /// scan ahead from the current position for whichever comes first at the top level, ignoring
    /// any content nested inside parentheses, brackets or functions (e.g. `calc(...)`, attribute
    /// selectors, `:is(...)`), which can never contain a top-level `{`.
    fn starts_nested_rule(&mut self) -> bool {
        let mut depth: usize = 0;
        let mut offset = 0;

        loop {
            let t = self.tokenizer.lookahead(offset);
            match t.token_type {
                TokenType::LParen | TokenType::LBracket | TokenType::Function(_) => depth += 1,
                TokenType::RParen | TokenType::RBracket => depth = depth.saturating_sub(1),
                TokenType::LCurly if depth == 0 => return true,
                TokenType::Semicolon | TokenType::RCurly if depth == 0 => return false,
                TokenType::Eof => return false,
                _ => {}
            }
            offset += 1;
        }
    }

    fn parse_consume_declaration(&mut self) -> CssResult<Option<Node>> {
        log::trace!("parse_consume_declaration");

        match self.parse_declaration()? {
            Some(declaration) => Ok(Some(declaration)),
            None => Ok(None),
        }
    }

    /// Reads until the end of a declaration or rule (or end of the block), in case there is a syntax error
    pub(crate) fn parse_until_rule_end(&mut self) {
        while let Ok(t) = self.consume_any() {
            match t.token_type {
                TokenType::Semicolon => {
                    break;
                }
                TokenType::RCurly => {
                    self.tokenizer.reconsume();
                    break;
                }
                TokenType::Eof => {
                    break;
                }
                _ => {
                    // ignore
                }
            }
        }
    }

    pub fn parse_block(&mut self, mode: BlockParseMode) -> CssResult<Node> {
        log::trace!("parse_block with parse mode: {mode:?}");

        let loc = self.tokenizer.current_location();
        let mut children: Vec<Node> = Vec::new();
        let mut semicolon_seperated = true;

        while !self.tokenizer.eof() {
            let t = self.consume_any()?;
            match t.token_type {
                TokenType::RCurly => {
                    // End the block
                    self.tokenizer.reconsume();

                    let n = Node::new(NodeType::Block { children }, t.location);
                    return Ok(n);
                }
                TokenType::Whitespace(_) | TokenType::Comment(_) => {
                    // just eat the token
                }

                TokenType::AtKeyword(_) => {
                    self.tokenizer.reconsume();
                    if let Some(at_rule_node) = self.parse_at_rule(mode == BlockParseMode::StyleBlock)? {
                        children.push(at_rule_node);
                    }
                    semicolon_seperated = false;
                    continue;
                }
                TokenType::Semicolon => {
                    semicolon_seperated = true;
                }
                _ => match mode {
                    BlockParseMode::StyleBlock => {
                        if !semicolon_seperated {
                            return Err(CssError::with_location(
                                format!("Expected a ; got {t:?}").as_str(),
                                self.tokenizer.current_location(),
                            ));
                        }

                        self.tokenizer.reconsume();
                        if self.starts_nested_rule() {
                            // CSS Nesting: a nested style rule (with or without a leading `&`).
                            if let Some(rule_node) = self.parse_consume_rule()? {
                                children.push(rule_node);
                            }
                            // A nested rule is self-terminating (it ends with `}`), so no
                            // separating semicolon is required before the next item.
                            semicolon_seperated = true;
                        } else {
                            if let Some(declaration_node) = self.parse_consume_declaration()? {
                                children.push(declaration_node);
                            }
                            semicolon_seperated = false;
                        }
                    }
                    BlockParseMode::RegularBlock => {
                        self.tokenizer.reconsume();

                        if let Some(rule_node) = self.parse_consume_rule()? {
                            children.push(rule_node);
                        }

                        semicolon_seperated = false;
                    }
                },
            }
        }

        let n = Node::new(NodeType::Block { children }, loc);

        Ok(n)
    }
}
