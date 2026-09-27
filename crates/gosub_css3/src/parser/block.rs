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

    /// Reads until the end of a rule (or the end of the enclosing block), in case there is a
    /// syntax error.
    ///
    /// A rule is either a statement that ends at a top-level `;` (e.g. a block-less at-rule) or a
    /// construct whose body is a `{ ... }` block. To resynchronise at the next rule boundary we
    /// balance nested `()`, `[]`, `{}` and functions: a top-level `;` ends a block-less rule, and
    /// once we have entered and matched a `{ ... }` block we stop right after its closing `}`. A
    /// `}` seen while still at the top level belongs to an enclosing block we did not open, so we
    /// leave it in place for that block to consume.
    pub(crate) fn parse_until_rule_end(&mut self) {
        let mut depth: usize = 0;
        while let Ok(t) = self.consume_any() {
            match t.token_type {
                TokenType::LParen | TokenType::LBracket | TokenType::LCurly | TokenType::Function(_) => {
                    depth += 1;
                }
                TokenType::RParen | TokenType::RBracket => {
                    depth = depth.saturating_sub(1);
                }
                TokenType::RCurly => {
                    if depth == 0 {
                        // Closing brace of an enclosing block we never opened; leave it.
                        self.tokenizer.reconsume(t);
                        break;
                    }
                    depth -= 1;
                    if depth == 0 {
                        // Consumed the rule's whole `{ ... }` block.
                        break;
                    }
                }
                TokenType::Semicolon if depth == 0 => {
                    // Statement-level rule with no block.
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

    /// Discards the remnants of an invalid declaration inside a style block: consumes tokens up to
    /// and including the next top-level `;`, or up to (but not including) the block's closing `}`.
    /// Nested `()`, `[]`, `{}` and functions are balanced so a terminator inside them is not
    /// mistaken for the end of the declaration.
    fn skip_to_declaration_end(&mut self) {
        let mut depth: usize = 0;
        loop {
            let t = self.tokenizer.lookahead(0);
            match t.token_type {
                TokenType::Eof => break,
                TokenType::Semicolon if depth == 0 => {
                    self.tokenizer.consume(); // eat the terminator
                    break;
                }
                TokenType::RCurly if depth == 0 => break, // leave for the block to close
                TokenType::LParen | TokenType::LBracket | TokenType::LCurly | TokenType::Function(_) => {
                    depth += 1;
                }
                TokenType::RParen | TokenType::RBracket | TokenType::RCurly => {
                    depth = depth.saturating_sub(1);
                }
                _ => {}
            }
            self.tokenizer.consume();
        }
    }

    /// Blocks nest through `parse_block` -> `parse_at_rule`/`parse_rule` -> `parse_block`, so the
    /// body runs one recursion level deeper.
    ///
    /// A block past the depth cap is skipped whole, up to its closing `}`, and the rule that owns
    /// it is dropped. The rules around it are kept. Firefox and WebKit do the same. cssparser
    /// fails a block past 75 levels and the rule list skips that rule. WebKit drops a style rule
    /// past 128 levels. Before, recovery stopped short of the dropped rule's `}`, so each
    /// enclosing block closed one brace early and the rule after the whole nest was lost.
    pub fn parse_block(&mut self, mode: BlockParseMode) -> CssResult<Node> {
        if self.recursion_depth >= crate::MAX_RECURSION_DEPTH {
            // `recurse` refuses this block below. Skip it first so the owning rule can end.
            self.skip_block_contents();
            self.skipped_deep_block = true;
        }
        self.recurse(|parser| parser.parse_block_inner(mode))
    }

    /// Consumes the rest of the current block without parsing it, and stops before the `}` that
    /// closes it. Brackets are counted, not recursed into, so any depth costs no stack.
    fn skip_block_contents(&mut self) {
        let mut depth: usize = 0;
        loop {
            let t = self.tokenizer.lookahead(0);
            match t.token_type {
                TokenType::Eof => break,
                TokenType::RCurly if depth == 0 => break,
                TokenType::LParen | TokenType::LBracket | TokenType::LCurly | TokenType::Function(_) => {
                    depth += 1;
                }
                TokenType::RParen | TokenType::RBracket | TokenType::RCurly => {
                    depth = depth.saturating_sub(1);
                }
                _ => {}
            }
            self.tokenizer.consume();
        }
    }

    /// Error recovery for a rule that failed to parse. When its block was skipped for being too
    /// deep, the parser sits at that block's `}`, so consuming it ends the rule. Otherwise skip
    /// to the end of the rule.
    pub(crate) fn recover_from_rule_error(&mut self) {
        if std::mem::take(&mut self.skipped_deep_block) {
            if self.tokenizer.lookahead(0).token_type == TokenType::RCurly {
                self.tokenizer.consume();
            }
            return;
        }
        self.parse_until_rule_end();
    }

    fn parse_block_inner(&mut self, mode: BlockParseMode) -> CssResult<Node> {
        log::trace!("parse_block with parse mode: {mode:?}");

        let loc = self.tokenizer.current_location();
        let mut children: Vec<Node> = Vec::new();
        let mut semicolon_seperated = true;

        while !self.tokenizer.eof() {
            let t = self.consume_any()?;
            match t.token_type {
                TokenType::RCurly => {
                    // End the block
                    let n = Node::new(NodeType::Block { children }, t.location);
                    self.tokenizer.reconsume(t);
                    return Ok(n);
                }
                TokenType::Whitespace(_) | TokenType::Comment(_) => {
                    // discard and keep consuming
                }

                TokenType::AtKeyword(_) => {
                    self.tokenizer.reconsume(t);
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
                            if self.config.ignore_errors {
                                // An invalid declaration left junk before the next `;`/`}`
                                // (e.g. a missing semicolon glued two declarations together).
                                // Per the CSS spec, discard the bad declaration and keep parsing
                                // the remaining declarations in this block rather than aborting
                                // the whole rule, which would desync block boundaries.
                                log::warn!("Ignoring error in parse_block: Expected a ; got {t:?}");
                                self.tokenizer.reconsume(t);
                                self.skip_to_declaration_end();
                                semicolon_seperated = true;
                                continue;
                            }
                            return Err(CssError::with_location(
                                format!("Expected a ; got {t:?}").as_str(),
                                self.tokenizer.current_location(),
                            ));
                        }

                        self.tokenizer.reconsume(t);
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
                        self.tokenizer.reconsume(t);

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

#[cfg(test)]
mod tests {
    use crate::walker::Walker;
    use crate::MAX_RECURSION_DEPTH;
    use crate::{CssOrigin, ParserConfig};
    use gosub_shared::byte_stream::{ByteStream, Encoding};

    /// Parses a full stylesheet with error recovery enabled and returns the walked AST.
    fn parse_recovering(input: &str) -> String {
        let mut stream = ByteStream::from_str(input, Encoding::UTF8);
        let config = ParserConfig {
            ignore_errors: true,
            ..Default::default()
        };
        let mut parser = crate::Css3::new(&mut stream, config, CssOrigin::Author, "");
        let result = parser.parse_stylesheet_internal().unwrap().unwrap();
        Walker::new(&result).walk_to_string()
    }

    #[test]
    fn recover_missing_semicolon_keeps_block_and_following_rules() {
        // A declaration is missing its terminating `;` (`color: red` glued to `width: 1px`).
        // The bad declaration is dropped, but the rest of the block and the following rule must
        // still parse instead of cascading into desync.
        let out = parse_recovering(".a { color: red width: 1px; height: 2px; } .b { top: 0 }");

        // The valid declarations after the recovery point survive.
        assert!(out.contains("property: height"), "expected height to survive:\n{out}");
        // The following rule is not swallowed by the cascade.
        assert!(out.contains("[ClassSelector] b"), "expected .b rule to parse:\n{out}");
        assert!(out.contains("property: top"), "expected .b's declaration:\n{out}");
    }

    /// Parses a stylesheet without error recovery, surfacing the first error.
    fn parse_strict(input: &str) -> gosub_shared::errors::CssResult<Option<String>> {
        let mut stream = ByteStream::from_str(input, Encoding::UTF8);
        let mut parser = crate::Css3::new(&mut stream, ParserConfig::default(), CssOrigin::Author, "");
        Ok(parser
            .parse_stylesheet_internal()?
            .map(|node| Walker::new(&node).walk_to_string()))
    }

    /// `n` levels of `@media screen{ ... }` wrapped around a single declaration.
    fn nested_media(n: usize) -> String {
        format!("{}a{{color:red}}{}", "@media screen{".repeat(n), "}".repeat(n))
    }

    /// Number of nested `@media` levels the parser kept in the AST.
    fn media_levels(ast: &str) -> usize {
        ast.matches("[AtRule] name: media").count()
    }

    #[test]
    fn block_nesting_at_the_limit_is_kept_whole() {
        // The innermost `a{...}` is a style rule whose own block costs a level, so the deepest
        // document that survives intact has one fewer `@media` than the cap.
        let ast = parse_strict(&nested_media(MAX_RECURSION_DEPTH - 1))
            .expect("within the cap")
            .expect("expected an AST");

        assert_eq!(media_levels(&ast), MAX_RECURSION_DEPTH - 1);
        assert!(ast.contains("property: color"), "innermost declaration lost:\n{ast}");
    }

    #[test]
    fn block_nesting_beyond_the_limit_is_an_error() {
        let err = parse_strict(&nested_media(MAX_RECURSION_DEPTH + 1)).expect_err("expected a depth error");

        assert!(err.to_string().contains("nesting too deep"), "unexpected error: {err}");
    }

    #[test]
    fn block_nesting_beyond_the_limit_is_truncated_when_recovering() {
        // With error recovery each enclosing at-rule absorbs the depth error and keeps its own
        // (now empty) block, so the document is truncated at the cap rather than rejected.
        let ast = parse_recovering(&nested_media(MAX_RECURSION_DEPTH + 1));

        assert_eq!(media_levels(&ast), MAX_RECURSION_DEPTH);
        assert!(
            !ast.contains("property: color"),
            "content past the cap should be discarded:\n{ast}"
        );
    }

    #[test]
    fn pathological_block_nesting_does_not_overflow_the_stack() {
        // Without the depth cap this recursed one stack frame per level and aborted the process
        // with a stack overflow -- an abort that no `Result` or `catch_unwind` can intercept.
        // Runs on a test thread's 2 MiB stack, the same size the engine's workers get.
        assert!(parse_strict(&nested_media(50_000)).is_err());

        // The recovering path walks the same depth without unwinding, so exercise it too.
        assert_eq!(
            media_levels(&parse_recovering(&nested_media(50_000))),
            MAX_RECURSION_DEPTH
        );
    }

    #[test]
    fn recover_invalid_rule_skips_whole_block() {
        // The first rule's body is malformed enough to abort the rule; recovery must skip its
        // entire `{ ... }` block (brace-balanced) so the following rule parses cleanly.
        let out = parse_recovering(".bad { @#$ {nested junk} } .good { color: blue }");

        assert!(
            out.contains("[ClassSelector] good"),
            "expected .good rule to parse:\n{out}"
        );
        assert!(out.contains("property: color"), "expected .good's declaration:\n{out}");
    }

    /// A rule nested past the cap is dropped, and only that rule. Recovery used to stop before
    /// the dropped rule's `}`, so every enclosing block closed one brace early and the rule
    /// after the nest was read as part of it and lost.
    #[test]
    fn a_rule_past_the_cap_is_dropped_and_its_neighbours_kept() {
        let depth = MAX_RECURSION_DEPTH + 5;
        let cases = [
            (
                "@media",
                format!(
                    ".before{{top:1px}} {}.deep{{top:2px}}{} .after{{top:3px}}",
                    "@media screen{".repeat(depth),
                    "}".repeat(depth)
                ),
            ),
            (
                "&",
                format!(
                    ".before{{top:1px}} {}top:2px{} .after{{top:3px}}",
                    ".n{&.m{".repeat(depth / 2),
                    "}}".repeat(depth / 2)
                ),
            ),
        ];
        for (kind, css) in cases {
            let out = parse_recovering(&css);
            assert!(out.contains("[ClassSelector] before"), "{kind}: lost .before:\n{out}");
            assert!(out.contains("[ClassSelector] after"), "{kind}: lost .after:\n{out}");
            assert!(
                !out.contains("[ClassSelector] deep"),
                "{kind}: kept the rule past the cap"
            );
            assert_eq!(out.matches("property: top").count(), 2, "{kind}:\n{out}");
        }
    }

    /// Inside the nest, the rule next to the dropped one stays in its own block.
    #[test]
    fn a_sibling_of_a_rule_past_the_cap_stays_in_its_block() {
        let css = format!(
            "@media print{{ {}.deep{{top:1px}}{} .keep{{top:2px}} }} .after{{top:3px}}",
            "@media screen{".repeat(MAX_RECURSION_DEPTH + 5),
            "}".repeat(MAX_RECURSION_DEPTH + 5)
        );
        let out = parse_recovering(&css);
        let keep = out.find("[ClassSelector] keep").expect(".keep lost");
        let after = out.find("[ClassSelector] after").expect(".after lost");
        // `.keep` is still inside `@media print`, so it is indented further than `.after`.
        let indent = |at: usize| at - out[..at].rfind('\n').map_or(0, |n| n + 1);
        assert!(indent(keep) > indent(after), "{out}");
    }

    /// After a rule past the cap, a later broken rule still skips its whole block.
    #[test]
    fn a_later_broken_rule_still_recovers_after_a_skipped_one() {
        let depth = MAX_RECURSION_DEPTH + 5;
        let css = format!(
            "{}.deep{{top:1px}}{} .x{{--v:{}1{}}} .b{{top:2px}} .bad{{ @#$ {{junk}} }} .good{{top:3px}}",
            "@media screen{".repeat(depth),
            "}".repeat(depth),
            "{".repeat(depth),
            "}".repeat(depth)
        );
        let out = parse_recovering(&css);
        assert!(out.contains("[ClassSelector] b"), "{out}");
        assert!(out.contains("[ClassSelector] good"), "{out}");
    }

    /// A value nested past the cap drops that declaration only.
    #[test]
    fn a_value_past_the_cap_drops_only_its_declaration() {
        let depth = MAX_RECURSION_DEPTH + 5;
        let css = format!(
            ".a{{color:red;width:{}1px{};height:2px}} .after{{top:3px}}",
            "calc(".repeat(depth),
            ")".repeat(depth)
        );
        let out = parse_recovering(&css);
        assert!(out.contains("property: color"), "{out}");
        assert!(!out.contains("property: width"), "{out}");
        assert!(out.contains("property: height"), "{out}");
        assert!(out.contains("[ClassSelector] after"), "{out}");
    }
}
