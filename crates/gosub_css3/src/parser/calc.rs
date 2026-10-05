use crate::node::{Node, NodeType};
use crate::tokenizer::TokenType;
use crate::Css3;
use gosub_shared::errors::{CssError, CssResult};

impl Css3<'_> {
    pub fn parse_calc(&mut self) -> CssResult<Node> {
        log::trace!("parse_calc");

        let loc = self.tokenizer.current_location();

        let tokens = self.parse_calc_tokens()?;

        Ok(Node::new(NodeType::Calc { tokens }, loc))
    }

    /// Read a `calc()` body up to its closing `)`, as a flat list of token nodes.
    ///
    /// `calc()` needs a parse path of its own because the generic value parser has no arm for a
    /// bare `(`, and a `calc()` body is full of them. What it used to do with the tokens it read
    /// was rebuild them into a string, which the evaluator then tokenized all over again. That
    /// hop lost information: whitespace runs collapsed, comments vanished with nothing put in
    /// their place, and a dimension printed without the sign the tokenizer had folded into it,
    /// so `calc(1px +2px)` was stored as `calc(1px 2px)`.
    ///
    /// Keeping the tokens means the whitespace that css-values-4 §10.1 makes load-bearing
    /// survives on the operators that need it, and nothing is tokenized twice.
    pub(crate) fn parse_calc_tokens(&mut self) -> CssResult<Vec<Node>> {
        log::trace!("parse_calc_tokens");

        let mut tokens: Vec<Node> = Vec::new();
        let mut space = false;

        loop {
            let t = self.consume_any()?;
            let loc = t.location;

            match t.token_type {
                TokenType::Eof | TokenType::RParen => break,
                TokenType::Whitespace(_) => {
                    space = true;
                    continue;
                }
                TokenType::Comment(_) => continue,
                _ => {}
            }

            // Whitespace ahead of this token is also whitespace after whatever preceded it,
            // which is the half of the rule an operator cannot see for itself.
            if space {
                if let Some(NodeType::Operator { space_after, .. }) = tokens.last_mut().map(|node| &mut node.node_type)
                {
                    *space_after = true;
                }
            }

            let node_type = match t.token_type {
                // A nested function keeps its own argument list. The recursive call consumes
                // through the matching `)`.
                TokenType::Function(name) => NodeType::Function {
                    name,
                    arguments: self.recurse(Self::parse_calc_tokens)?,
                },
                // A parenthesized group is a call with no name: it groups exactly as `calc()`
                // does, which is what css-values-4 says, but it has to serialize back as `( )`
                // rather than as `calc( )` - the serialization suites check the difference.
                // Grouping then survives as structure and needs no parenthesis token of its own.
                TokenType::LParen => NodeType::Function {
                    name: String::new(),
                    arguments: self.recurse(Self::parse_calc_tokens)?,
                },
                TokenType::Number(value, kind) => NodeType::Number { value, kind },
                TokenType::Dimension { value, unit } => NodeType::Dimension { value, unit },
                TokenType::Percentage(value) => NodeType::Percentage { value },
                // An identifier is a numeric constant (`pi`, `e`, `infinity`, `NaN`) or a
                // keyword such as `no-clamp`. Which one is the evaluator's to decide.
                TokenType::Ident(value) => NodeType::Ident { value },
                TokenType::Comma => NodeType::Comma,
                TokenType::Delim(c @ ('+' | '-' | '*' | '/')) => NodeType::Operator {
                    value: c.to_string(),
                    space_before: space,
                    space_after: false,
                },
                other => {
                    return Err(CssError::with_location(
                        format!("Unexpected token in calc() body: {other:?}").as_str(),
                        self.tokenizer.current_location(),
                    ))
                }
            };

            tokens.push(Node::new(node_type, loc));
            space = false;
        }

        Ok(tokens)
    }
}

#[cfg(test)]
mod tests {
    use crate::stylesheet::CssValue;

    /// The body of `calc(<input>)` as the values it parses to.
    fn body(input: &str) -> Vec<CssValue> {
        crate::parse_calc_body(input).expect("calc body should parse")
    }

    #[test]
    fn a_body_keeps_its_tokens_rather_than_becoming_text() {
        assert_eq!(
            body("1px + 2px"),
            vec![
                CssValue::Unit(1.0, "px".to_string()),
                CssValue::String("+".to_string()),
                CssValue::Unit(2.0, "px".to_string()),
            ]
        );
    }

    #[test]
    fn a_sign_folded_into_a_number_survives() {
        // The tokenizer reads `+2px` as one dimension token. Printing it back out dropped the
        // sign - a positive `f32` has none - so the body reached the evaluator as `1px 2px` and
        // the round trip had quietly rewritten what the author wrote. Keeping the tokens means
        // there is nothing to print and nothing to lose: two adjacent values, which is what
        // `1px +2px` is and why css-values-4 makes it invalid.
        assert_eq!(
            body("1px +2px"),
            vec![
                CssValue::Unit(1.0, "px".to_string()),
                CssValue::Unit(2.0, "px".to_string()),
            ]
        );
    }

    #[test]
    fn a_parenthesised_group_is_an_unnamed_call() {
        // A group is a call with no name: it groups the way `calc()` does but serializes back
        // as `( )`. Shown with a `var()` inside, because a group this *can* reduce is reduced on
        // the way past - `(1px + 2px)` becomes `calc(3px)` before anyone can look at its shape.
        assert_eq!(
            body("(1px + var(--x))"),
            vec![CssValue::Function(
                String::new(),
                vec![
                    CssValue::Unit(1.0, "px".to_string()),
                    CssValue::String("+".to_string()),
                    CssValue::Function("var".to_string(), vec![CssValue::String("--x".to_string())]),
                ]
            )]
        );
    }

    #[test]
    fn a_nested_function_keeps_its_own_arguments() {
        // `min(1em, 21px)` cannot be folded before there is a font-size, so it survives as the
        // call it was - commas and all - to be finished at computed-value time.
        assert_eq!(
            body("min(1em, 21px)"),
            vec![CssValue::Function(
                "min".to_string(),
                vec![
                    CssValue::Unit(1.0, "em".to_string()),
                    CssValue::Comma,
                    CssValue::Unit(21.0, "px".to_string()),
                ]
            )]
        );
    }

    #[test]
    fn a_comment_inside_a_body_does_not_glue_its_neighbours() {
        // Comments used to be dropped with nothing put in their place, so `1px/**/+/**/2px`
        // rebuilt as `1px+2px`. Now they are simply not tokens, and the spacing either side of
        // them is recorded on the operator where the rule can see it.
        let values = body("1px/**/+/**/2px");
        assert_eq!(values.len(), 3);
        assert_eq!(values[1], CssValue::String("+".to_string()));
    }
}
