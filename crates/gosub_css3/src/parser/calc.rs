use crate::node::{Node, NodeType};
use crate::tokenizer::TokenType;
use crate::Css3;
use gosub_shared::errors::{CssError, CssResult};

/// Whether this token is a value rather than punctuation - a number, a length, a percentage or
/// a keyword.
///
/// Two of these cannot legally sit next to each other inside `calc()`: something has to join
/// them. The check matters because the body is rebuilt by concatenating token text, so a pair
/// with nothing between them would be spliced into a single different token - see
/// [`Css3::parse_calc_expr`].
fn is_value_token(token: &TokenType) -> bool {
    matches!(
        token,
        TokenType::Number(_) | TokenType::Dimension { .. } | TokenType::Percentage(_) | TokenType::Ident(_)
    )
}

impl Css3<'_> {
    pub fn parse_calc(&mut self) -> CssResult<Node> {
        log::trace!("parse_calc");

        let loc = self.tokenizer.current_location();

        let expr = self.parse_calc_expr()?;

        Ok(Node::new(NodeType::Calc { expr: Box::new(expr) }, loc))
    }

    fn parse_calc_expr(&mut self) -> CssResult<Node> {
        log::trace!("parse_calc_expr");

        let loc = self.tokenizer.current_location();

        // Rebuild the expression text from the consumed TOKENS. Slicing the raw stream
        // does not work here: the tokenizer pre-tokenizes into a buffer, so the stream
        // position runs ahead of the current token and the slice comes up empty.
        let mut expr = String::new();
        // The previous token, to catch two values written with nothing between them.
        let mut previous_was_value = false;

        loop {
            let t = self.consume_any()?;

            // `calc(10px+20px)` is invalid CSS: `+` and `-` are operators only with whitespace
            // around them, so here the `+` is read as the sign of `+20px` and the body is two
            // dimensions in a row. Rebuilding from token text would splice them into the single
            // token `10px20px` - a different value entirely, accepted without complaint because
            // nothing validates a calc body. Refuse instead of silently changing the meaning.
            let is_value = is_value_token(&t.token_type);
            if is_value && previous_was_value {
                return Err(CssError::with_location(
                    "Two values in a row inside calc(): an operator needs whitespace around it",
                    self.tokenizer.current_location(),
                ));
            }
            previous_was_value = is_value;

            match t.token_type {
                TokenType::Eof => break,
                // A nested function or `(` opens a sub-expression, one recursion level
                // deeper. The recursive call consumes through the matching `)`.
                TokenType::Function(name) => {
                    expr.push_str(&name);
                    expr.push('(');
                    let inner = self.recurse(Self::parse_calc_expr)?;
                    if let NodeType::Raw { value } = inner.node_type {
                        expr.push_str(&value);
                    }
                    expr.push(')');
                }
                TokenType::LParen => {
                    expr.push('(');
                    let inner = self.recurse(Self::parse_calc_expr)?;
                    if let NodeType::Raw { value } = inner.node_type {
                        expr.push_str(&value);
                    }
                    expr.push(')');
                }
                TokenType::RParen => break,
                TokenType::Comment(_) => {}
                _ => {
                    // Token's Display form is its CSS serialization (whitespace -> " ").
                    expr.push_str(&t.to_string());
                }
            }
        }

        Ok(Node::new(
            NodeType::Raw {
                value: expr.trim().to_string(),
            },
            loc,
        ))
    }
}

#[cfg(test)]
mod tests {
    use crate::stylesheet::{CssStylesheet, CssValue};
    use gosub_interface::css3::CssOrigin;
    use gosub_shared::config::ParserConfig;

    /// Parse `width: <value>` and return the stored calc body, or `None` when the declaration
    /// did not survive.
    fn calc_body(value: &str) -> Option<String> {
        let config = ParserConfig {
            ignore_errors: true,
            ..Default::default()
        };
        let sheet: CssStylesheet =
            crate::Css3::parse_str(&format!("*{{width:{value}}}"), config, CssOrigin::Author, "").ok()?;
        let declaration = sheet.rules.first()?.declarations().first()?;
        match &declaration.value {
            CssValue::Function(name, args) if name == "calc" => match args.first() {
                Some(CssValue::String(body)) => Some(body.clone()),
                _ => None,
            },
            _ => None,
        }
    }

    #[test]
    fn an_operator_without_whitespace_is_refused() {
        // `+` and `-` are operators only with whitespace around them, so the `+` here is read as
        // the sign of `+20px` and the body is two dimensions in a row. The body is rebuilt from
        // token text, so keeping it would splice those into `10px20px` - a different value, and
        // one nothing would complain about, since no one validates a calc body.
        assert_eq!(calc_body("calc(10px+20px)"), None, "must not be kept as 10px20px");
        assert_eq!(calc_body("calc(2+3)"), None);
    }

    #[test]
    fn well_formed_expressions_are_kept_verbatim() {
        // The regression the check above must not cause: ordinary arithmetic still parses. `*`
        // and `/` need no whitespace, and a signed leading operand is one token, not two.
        for (value, expected) in [
            ("calc(10px + 20px)", "10px + 20px"),
            ("calc(100% - 10px)", "100% - 10px"),
            ("calc(2 * 3)", "2 * 3"),
            ("calc(2*3)", "2*3"),
            ("calc(-1px + 2px)", "-1px + 2px"),
            ("calc((1px + 2px) * 3)", "(1px + 2px) * 3"),
            ("calc(min(1px, 2px) + 3px)", "min(1px, 2px) + 3px"),
            ("calc(1px)", "1px"),
        ] {
            assert_eq!(calc_body(value).as_deref(), Some(expected), "for {value}");
        }
    }
}
