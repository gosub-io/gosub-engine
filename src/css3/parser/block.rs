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

    pub fn parse_block(&mut self, mode: BlockParseMode) -> Result<Node, Error> {
        log::trace!("parse_block");
        let mut children: Vec<Node> = Vec::new();

        let mut semicolon_seperated= true;

        while !self.tokenizer.eof() {
            let t = self.consume_any()?;
            match t.token_type {
                TokenType::RCurly => {
                    // End the block
                    self.tokenizer.reconsume();

                    let n = Node::new(NodeType::Block { children });
                    return Ok(n)
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
                        if semicolon_seperated == false {
                            return Err(Error::new(
                                format!("Expected a ; got {:?}", t),
                                self.tokenizer.current_location.clone(),
                            ));
                        }

                        self.tokenizer.reconsume();
                        if t.is_delim('&') {
                            children.push(self.parse_consume_rule()?);
                        } else {
                            children.push(self.parse_consume_declaration()?);
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
                        children.push(self.parse_consume_rule()?);

                        semicolon_seperated = false;
                    }
                },
            }
        }

        let n = Node::new(NodeType::Block { children });

        Ok(n)
    }
}
