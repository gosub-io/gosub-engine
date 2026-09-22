use crate::node::{Node, NodeType};
use crate::tokenizer::TokenType;
use crate::Css3;
use gosub_shared::errors::CssResult;

impl Css3<'_> {
    /// Parse the sheet, handing each top-level node to `emit` as soon as it is complete.
    ///
    /// The caller can convert a rule and drop its nodes before the next one is parsed, which is
    /// what keeps a large sheet's AST from existing all at once: on a 2.2 MB sheet the tree is
    /// some 460,000 nodes at 104 bytes each, and it used to be held in full while the whole of
    /// it was converted. Returns whether anything was emitted.
    pub fn parse_stylesheet_streaming<F>(&mut self, mut emit: F) -> CssResult<bool>
    where
        F: FnMut(Node),
    {
        log::trace!("parse_stylesheet");

        let mut emitted = false;
        let mut children = Emitter {
            emit: &mut emit,
            emitted: &mut emitted,
        };

        while !self.tokenizer.eof() {
            let t = self.consume_any()?;

            match t.token_type {
                TokenType::Eof => {}
                TokenType::Whitespace(_) => {}
                TokenType::Comment(comment) => {
                    if comment.chars().nth(2) == Some('!') {
                        children.push(Node::new(NodeType::Comment { value: comment }, t.location));
                    }
                }
                TokenType::Cdo => {
                    children.push(Node::new(NodeType::Cdo, t.location));
                }
                TokenType::Cdc => {
                    children.push(Node::new(NodeType::Cdc, t.location));
                }
                TokenType::AtKeyword(_) => {
                    self.tokenizer.reconsume(t);

                    let at_rule = self.parse_at_rule(false)?;
                    if let Some(at_rule_node) = at_rule {
                        children.push(at_rule_node);
                    }
                }
                _ => {
                    self.tokenizer.reconsume(t);

                    let rule = self.parse_rule()?;
                    if let Some(rule_node) = rule {
                        children.push(rule_node);
                    }
                }
            }
        }

        Ok(emitted)
    }

    /// The whole sheet as one AST node, for the callers that want the tree rather than the
    /// rules: block parsing and the parser tests.
    pub fn parse_stylesheet_internal(&mut self) -> CssResult<Option<Node>> {
        let loc = self.tokenizer.current_location();
        let mut children = Vec::new();
        if !self.parse_stylesheet_streaming(|node| children.push(node))? {
            return Ok(None);
        }
        Ok(Some(Node::new(NodeType::StyleSheet { children }, loc)))
    }
}

/// Stands in for the `Vec` the loop used to push into, so the loop body reads the same whether
/// its nodes are being collected into a tree or converted and dropped one at a time.
struct Emitter<'a> {
    emit: &'a mut dyn FnMut(Node),
    emitted: &'a mut bool,
}

impl Emitter<'_> {
    fn push(&mut self, node: Node) {
        *self.emitted = true;
        (self.emit)(node);
    }
}
