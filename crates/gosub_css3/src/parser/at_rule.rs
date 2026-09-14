mod container;
mod font_face;
mod import;
mod layer;
mod media;
mod nest;
mod page;
mod scope;
mod starting_style;
mod supports;

use crate::node::{Node, NodeType};
use crate::parser::block::BlockParseMode;
use crate::tokenizer::TokenType;
use crate::Css3;
use cow_utils::CowUtils;
use gosub_shared::errors::{CssError, CssResult};

impl Css3<'_> {
    /// Decide how to read the body of an at-rule the parser has no specific handler for, by
    /// looking ahead for whichever comes first: a `{` or an `@` means the body holds nested rules
    /// (`@keyframes`), and the block's own `}` means it holds declarations (`@counter-style`,
    /// `@property`).
    ///
    /// Every arm used to answer `RegularBlock`, so a descriptor body was always parsed as if it
    /// were rules: `@counter-style x{system:numeric;…}` read `system:numeric` as a selector and
    /// then failed on the `;`, and the same for `@property`. The caller has already consumed the
    /// opening `{`, so the scan starts at the first token inside the block.
    fn declaration_block_at_rule(&mut self) -> BlockParseMode {
        let mut offset = 0;
        loop {
            let t = self.tokenizer.lookahead(offset);
            offset += 1;

            match t.token_type {
                // The block closed without a nested rule starting: it was all declarations.
                TokenType::RCurly => {
                    return BlockParseMode::StyleBlock;
                }
                TokenType::LCurly | TokenType::AtKeyword(_) => {
                    return BlockParseMode::RegularBlock;
                }
                TokenType::Eof => {
                    return BlockParseMode::RegularBlock;
                }
                _ => {
                    // continue
                }
            }
        }
    }

    /// Parses the prelude of a `@custom-selector` rule (CSS Extensions / PostCSS):
    /// `@custom-selector <custom-selector> <selector-list>;`, e.g.
    /// `@custom-selector :--heading h1, h2, h3;`. The `<custom-selector>` name is a
    /// pseudo-class-style `:--ident`.
    fn parse_at_rule_custom_selector_prelude(&mut self) -> CssResult<Node> {
        log::trace!("parse_at_rule_custom_selector_prelude");

        let loc = self.tokenizer.current_location();

        // <custom-selector> name: `:--ident`
        self.consume(TokenType::Colon)?;
        let name = self.consume_any_ident()?;
        self.consume_whitespace_comments();

        // <selector-list>
        let selectors = self.parse_selector_list()?;

        Ok(Node::new(
            NodeType::Value {
                children: vec![
                    Node::new(
                        NodeType::Ident {
                            value: format!(":{name}"),
                        },
                        loc,
                    ),
                    selectors,
                ],
            },
            loc,
        ))
    }

    fn read_sequence_at_rule_prelude(&mut self) -> CssResult<Node> {
        log::trace!("read_sequence_at_rule_prelude");

        let loc = self.tokenizer.lookahead(0).location;

        Ok(Node::new(
            NodeType::Container {
                children: self.parse_value_sequence()?,
            },
            loc,
        ))
    }

    fn parse_at_rule_prelude(&mut self, name: String) -> CssResult<Option<Node>> {
        log::trace!("parse_at_rule_prelude");

        self.consume_whitespace_comments();
        let node = match name.cow_to_lowercase().as_ref() {
            "container" => Some(self.parse_at_rule_container_prelude()?),
            "custom-media" => Some(self.parse_at_rule_custom_media_prelude()?),
            "custom-selector" => Some(self.parse_at_rule_custom_selector_prelude()?),
            "font-face" => None,
            "import" => Some(self.parse_at_rule_import_prelude()?),
            "layer" => Some(self.parse_at_rule_layer_prelude()?),
            "media" => Some(self.parse_at_rule_media_prelude()?),
            "nest" => Some(self.parse_at_rule_nest_prelude()?),
            "page" => Some(self.parse_at_rule_page_prelude()?),
            "scope" => Some(self.parse_at_rule_scope_prelude()?),
            "starting-style" => None,
            "supports" => Some(self.parse_at_rule_supports_prelude()?),
            _ => Some(self.read_sequence_at_rule_prelude()?),
        };

        self.consume_whitespace_comments();

        let at_eof = self.tokenizer.eof();
        let t = self.tokenizer.lookahead(0);
        if !at_eof && t.token_type != TokenType::Semicolon && t.token_type != TokenType::LCurly {
            return Err(CssError::with_location(
                "Expected semicolon or left curly brace",
                t.location,
            ));
        }

        Ok(node)
    }

    fn parse_at_rule_block(&mut self, name: String, is_declaration: bool) -> CssResult<Option<Node>> {
        log::trace!("parse_at_rule_block");

        let t = self.tokenizer.consume();
        if t.token_type != TokenType::LCurly {
            // Seems there is no block
            return Ok(None);
        }

        // @Todo: maybe this is the other way around. Need to verify this
        let mut mode = BlockParseMode::RegularBlock;
        if is_declaration {
            mode = BlockParseMode::StyleBlock;
        }

        // parse block. They may or may not have nested rules depending on the is_declaration and block type
        let node = match name.cow_to_lowercase().as_ref() {
            "container" => Some(self.parse_block(mode)?),
            "font-face" => Some(self.parse_block(BlockParseMode::StyleBlock)?),
            "import" => None,
            "layer" => Some(self.parse_block(BlockParseMode::RegularBlock)?),
            "media" => Some(self.parse_block(mode)?),
            "nest" => Some(self.parse_block(BlockParseMode::StyleBlock)?),
            "page" => Some(self.parse_block(BlockParseMode::StyleBlock)?),
            "scope" => Some(self.parse_block(mode)?),
            "starting-style" => Some(self.parse_block(mode)?),
            "supports" => Some(self.parse_block(mode)?),
            _ => {
                let mode = self.declaration_block_at_rule();
                Some(self.parse_block(mode)?)
            }
        };

        // if we did a block, we need to close it
        if node.is_some() {
            self.consume(TokenType::RCurly)?;
        }

        Ok(node)
    }

    // Either the at_rule parsing succeeds as a whole, or not. When not a valid at_rule is found, we
    // return None if the config.ignore_errors is set to true, otherwise this will return an Err
    // and is handled by the caller
    pub fn parse_at_rule(&mut self, is_declaration: bool) -> CssResult<Option<Node>> {
        log::trace!("parse_at_rule");

        match self.parse_at_rule_internal(is_declaration) {
            Ok(at_rule_node) => Ok(Some(at_rule_node)),
            Err(err) if self.config.ignore_errors => {
                self.parse_until_rule_end();
                log::warn!("Ignoring error in parse_at_rule: {err:?}");
                Ok(None)
            }
            Err(err) => Err(err),
        }
    }

    fn parse_at_rule_internal(&mut self, is_declaration: bool) -> CssResult<Node> {
        let name;

        let t = self.consume_any()?;
        if let TokenType::AtKeyword(keyword) = t.token_type {
            name = keyword;
        } else {
            return Err(CssError::with_location("Expected at keyword", t.location));
        }
        self.consume_whitespace_comments();

        let prelude = self.parse_at_rule_prelude(name.clone())?;
        self.consume_whitespace_comments();

        let block = self.parse_at_rule_block(name.clone(), is_declaration)?;
        self.consume_whitespace_comments();

        Ok(Node::new(
            NodeType::AtRule {
                name: name.clone(),
                prelude: prelude.map(Box::new),
                block: block.map(Box::new),
            },
            t.location,
        ))
    }
}

#[cfg(test)]
mod tests {
    use crate::{Css3, CssOrigin};
    use gosub_shared::config::ParserConfig;

    /// Parse with errors *not* ignored, so a rule the parser trips over is visible as a failure
    /// rather than a log line. A page is parsed with `ignore_errors: true`, where the same trip
    /// only costs the offending rule - which is why this went unnoticed as five warnings a load.
    fn parse_strict(css: &str) -> Result<crate::stylesheet::CssStylesheet, String> {
        Css3::parse_str(css, ParserConfig::default(), CssOrigin::Author, "test.css").map_err(|e| format!("{e:?}"))
    }

    fn selector_text(sheet: &crate::stylesheet::CssStylesheet) -> Vec<String> {
        sheet
            .rules
            .iter()
            .flat_map(|rule| rule.selectors().iter().map(|sel| format!("{sel:?}")))
            .collect()
    }

    #[test]
    fn an_unknown_at_rule_body_of_descriptors_parses() {
        // Wikipedia ships three of these. Every arm of the block-mode scan answered "nested
        // rules", so `system:numeric` was read as a selector and the `;` after it was a parse
        // error - one per descriptor list, on every page load.
        let sheet = parse_strict("@counter-style meetei{system:numeric;symbols:'0' '1';suffix:') '}")
            .expect("a descriptor body should parse");
        assert!(
            selector_text(&sheet).is_empty(),
            "the at-rule contributes no style rules"
        );
    }

    #[test]
    fn an_unknown_at_rule_of_descriptors_leaves_its_neighbours_alone() {
        let sheet = parse_strict("a{color:red}@property --x{syntax:'<color>';inherits:false}b{color:blue}")
            .expect("a descriptor body should parse");
        let selectors = selector_text(&sheet);
        assert_eq!(selectors.len(), 2, "{selectors:?}");
    }

    #[test]
    fn an_unknown_at_rule_of_nested_rules_still_reads_as_rules() {
        // `@keyframes` is the other shape: its body holds qualified rules, not descriptors, and
        // the `{` of the first one is what says so.
        let sheet = parse_strict("@keyframes spin{from{opacity:0}to{opacity:1}}a{color:red}")
            .expect("a nested-rule body should parse");
        assert_eq!(selector_text(&sheet).len(), 1);
    }

    #[test]
    fn an_empty_unknown_at_rule_block_is_harmless() {
        let sheet = parse_strict("@counter-style empty{}a{color:red}").expect("an empty body should parse");
        assert_eq!(selector_text(&sheet).len(), 1);
    }
}
