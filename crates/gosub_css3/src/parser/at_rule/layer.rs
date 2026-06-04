use crate::node::{Node, NodeType};
use crate::tokenizer::TokenType;
use crate::Css3;
use gosub_shared::errors::CssResult;

impl Css3<'_> {
    // Parse a single layer name, which is <ident> ('.' <ident>)*
    // e.g. "base", "framework.utilities", "default.theme.dark"
    fn parse_layer_query(&mut self) -> CssResult<Node> {
        let loc = self.tokenizer.current_location();

        let first = self.consume_any_ident()?;
        let mut name = first;

        // Consume any dotted suffix parts
        loop {
            let la = self.tokenizer.lookahead(0);
            if !matches!(la.token_type, TokenType::Delim('.')) {
                break;
            }
            self.consume_any()?; // consume '.'
            match self.tokenizer.consume().token_type {
                TokenType::Ident(part) => {
                    name.push('.');
                    name.push_str(&part);
                }
                _ => {
                    self.tokenizer.reconsume();
                    break;
                }
            }
        }

        Ok(Node::new(NodeType::Ident { value: name }, loc))
    }

    pub fn parse_at_rule_layer_prelude(&mut self) -> CssResult<Node> {
        log::trace!("parse_at_rule_layer_prelude");

        let loc = self.tokenizer.current_location();

        self.consume_whitespace_comments();

        let mut layers = vec![];

        // Anonymous @layer { } has an empty prelude; stop before '{' or ';'
        while !self.tokenizer.eof() {
            let la = self.tokenizer.lookahead(0);
            if matches!(
                la.token_type,
                TokenType::LCurly | TokenType::Semicolon | TokenType::Eof
            ) {
                break;
            }

            let layer = self.parse_layer_query()?;
            layers.push(layer);

            self.consume_whitespace_comments();

            let t = self.consume_any()?;
            if !t.is_comma() {
                self.tokenizer.reconsume();
                break;
            }

            self.consume_whitespace_comments();
        }

        Ok(Node::new(NodeType::LayerList { layers }, loc))
    }
}
