use crate::node::{Node, NodeType};
use crate::tokenizer::TokenType;
use crate::{Css3, Error};

#[derive(Debug, PartialEq)]
pub enum BlockParseMode {
    StyleBlock,
    RegularBlock,
}

impl Css3<'_> {
    fn parse_consume_rule(&mut self) -> Result<Option<Node>, Error> {
        log::trace!("parse_consume_rule");
        self.parse_rule()
    }

    fn parse_consume_declaration(&mut self) -> Result<Option<Node>, Error> {
        log::trace!("parse_consume_declaration");

        match self.parse_declaration()? {
            Some(declaration) => Ok(Some(declaration)),
            None => Ok(None),
        }
    }

    /// Reads until the end of a declaration or rule (or end of the block), in case there is a syntax error
    pub(crate) fn parse_until_rule_end(&mut self) {
        loop {
            let t = self.consume_any();
            if t.is_err() {
                break;
            }
            match t.unwrap().token_type {
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

    pub fn parse_block(&mut self, mode: BlockParseMode) -> Result<Node, Error> {
        log::trace!("parse_block with parse mode: {:?}", mode);

        let loc = self.tokenizer.current_location();
        let mut children: Vec<Node> = Vec::new();
        let mut semicolon_seperated = true;

        while !self.tokenizer.eof() {
            let t = self.consume_any()?;
            match t.token_type {
                TokenType::RCurly => {
                    // End the block
                    self.tokenizer.reconsume();

                    let n = Node::new(NodeType::Block { children }, t.location.clone());
                    return Ok(n);
                }
                TokenType::Whitespace(_) | TokenType::Comment(_) => {
                    // just eat the token
                }

                TokenType::AtKeyword(_) => {
                    self.tokenizer.reconsume();
                    if let Some(at_rule_node) =
                        self.parse_at_rule(mode == BlockParseMode::StyleBlock)?
                    {
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
                            return Err(Error::new(
                                format!("Expected a ; got {:?}", t),
                                self.tokenizer.current_location(),
                            ));
                        }

                        self.tokenizer.reconsume();
                        if t.is_delim('&') {
                            let rule = self.parse_consume_rule()?;
                            if let Some(rule_node) = rule {
                                children.push(rule_node);
                            }
                        } else {
                            let declaration = self.parse_consume_declaration()?;
                            if let Some(declaration_node) = declaration {
                                children.push(declaration_node);
                            }
                        }

                        // // check for either semicolon, eof, or rcurly
                        // let t = self.tokenizer.lookahead_sc(0);
                        // if t.token_type == TokenType::Semicolon {
                        //     self.consume(TokenType::Semicolon)?;
                        //     semicolon_seperated = true;
                        // } else if t.token_type == TokenType::RCurly {
                        //     self.tokenizer.reconsume();
                        //     break;
                        // }

                        semicolon_seperated = false;
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
