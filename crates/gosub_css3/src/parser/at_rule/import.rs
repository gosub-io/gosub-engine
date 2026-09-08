use crate::node::{Node, NodeType};
use crate::tokenizer::TokenType;
use crate::Css3;
use gosub_shared::errors::{CssError, CssResult};

impl Css3<'_> {
    pub fn parse_at_rule_import_prelude(&mut self) -> CssResult<Node> {
        log::trace!("parse_at_rule_import");

        let mut children = Vec::new();

        let loc = self.tokenizer.current_location();

        let t = self.consume_any()?;
        match t.token_type {
            TokenType::QuotedString(value) => {
                children.push(Node::new(NodeType::String { value }, loc));
            }
            TokenType::Url(url) => {
                children.push(Node::new(NodeType::Url { url }, loc));
            }
            TokenType::Function(ref name) if name.eq_ignore_ascii_case("url") => {
                self.tokenizer.reconsume(t);
                children.push(self.parse_url()?);
            }
            _ => {
                return Err(CssError::with_location(
                    "Expected string or url()",
                    self.tokenizer.current_location(),
                ));
            }
        }

        self.consume_whitespace_comments();

        // Optional `layer` / `layer(name)`. The bare keyword has to be consumed, not just
        // peeked at: leaving it in the stream made every `@import ... layer;` fail the
        // "expected semicolon" check in `parse_at_rule_prelude` and drop the whole rule.
        let bare_layer = match &self.tokenizer.lookahead_sc(0).token_type {
            TokenType::Ident(value) if value.eq_ignore_ascii_case("layer") => Some(value.clone()),
            _ => None,
        };
        if let Some(value) = bare_layer {
            let location = self.tokenizer.current_location();
            self.consume_any()?;
            children.push(Node::new(NodeType::Ident { value }, location));
        } else if matches!(
            &self.tokenizer.lookahead_sc(0).token_type,
            TokenType::Function(name) if name.eq_ignore_ascii_case("layer")
        ) {
            children.push(self.parse_function()?);
        }

        self.consume_whitespace_comments();

        // Optional `supports(...)`. Captured as raw text rather than run through
        // `parse_function`, which chokes on the colon in `supports(display: grid)` and used to
        // take the entire `@import` down with it. The text goes to `SupportsCondition`, the
        // same evaluator `@supports` uses.
        if matches!(
            &self.tokenizer.lookahead_sc(0).token_type,
            TokenType::Function(name) if name.eq_ignore_ascii_case("supports")
        ) {
            let function = self.consume_any()?;
            let start = self.tokenizer.lookahead(0).location.offset;
            let mut depth = 1usize;
            let mut end = start;
            while !self.tokenizer.eof() {
                let token = self.tokenizer.consume();
                match token.token_type {
                    // A `Function` token is an identifier plus its opening parenthesis.
                    TokenType::Function(_) | TokenType::LParen => depth += 1,
                    TokenType::RParen => {
                        depth -= 1;
                        if depth == 0 {
                            end = token.location.offset;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            let value = self.tokenizer.slice(start, end.max(start));
            children.push(Node::new(NodeType::Raw { value }, function.location));
        }

        self.consume_whitespace_comments();

        // Optional trailing media query list: `@import url(a.css) screen and (min-width: 40em);`
        // Without this the leftover tokens failed the caller's semicolon check and the import
        // was discarded, which is the common form on real sites.
        if !self.tokenizer.eof()
            && !matches!(
                self.tokenizer.lookahead_sc(0).token_type,
                TokenType::Semicolon | TokenType::LCurly
            )
        {
            children.push(self.parse_media_query_list()?);
        }

        self.consume_whitespace_comments();

        Ok(Node::new(NodeType::ImportList { children }, loc))
    }
}
