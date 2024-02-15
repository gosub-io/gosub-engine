use crate::css3::node::{Node, NodeType};
use crate::css3::tokenizer::TokenType;
use crate::css3::{Css3, Error};

pub enum BlockParseMode {
    StyleBlock,
    RegularBlock,
}

impl Css3<'_> {
    fn parse_consume_rule(&mut self) -> Result<Node, Error> {
        log::trace!("parse_consume_rule");
        self.parse_rule()
    }

    fn parse_consume_declaration(&mut self) -> Result<Node, Error> {
        log::trace!("parse_consume_declaration");
        let declaration = self.parse_declaration()?;

        Ok(declaration)
    }

    /// Reads until the end of a declaration or rule (or end of the block), in case there is a syntax error
    fn parse_until_rule_end(&mut self) {
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
        log::trace!("parse_block");

        let loc = self.tokenizer.current_location().clone();
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
                TokenType::Whitespace | TokenType::Comment(_) => {
                    // just eat the token
                }

                TokenType::AtKeyword(_) => {
                    self.tokenizer.reconsume();
                    children.push(self.parse_at_rule(true)?);
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
                                self.tokenizer.current_location().clone(),
                            ));
                        }

                        self.tokenizer.reconsume();
                        if t.is_delim('&') {
                            let rule = self.parse_consume_rule();
                            if rule.is_err() {
                                self.parse_until_rule_end();
                                if self.config.ignore_errors {
                                    continue;
                                } else {
                                    return rule;
                                }
                            }
                            children.push(rule.unwrap());
                        } else {
                            let declaration = self.parse_consume_declaration();
                            if declaration.is_err() {
                                self.parse_until_rule_end();
                                if self.config.ignore_errors {
                                    continue;
                                } else {
                                    return declaration;
                                }
                            }

                            children.push(declaration.unwrap());
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

                        let rule = self.parse_consume_rule();
                        if rule.is_err() {
                            self.parse_until_rule_end();
                            if self.config.ignore_errors {
                                continue;
                            } else {
                                return rule;
                            }
                        }
                        children.push(rule.unwrap());

                        semicolon_seperated = false;
                    }
                },
            }
        }

        let n = Node::new(NodeType::Block { children }, loc);

        Ok(n)
    }
}
