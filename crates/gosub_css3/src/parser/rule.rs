use crate::node::{Node, NodeType};
use crate::parser::block::BlockParseMode;
use crate::tokenizer::TokenType;
use crate::{Css3, Error};

impl Css3<'_> {
    // Either the rule parsing succeeds as a whole, or not. When not a valid rule is found, we
    // return None if the config.ignore_errors is set to true, otherwise this will return an Err
    // and is handled by the caller
    pub fn parse_rule(&mut self) -> Result<Option<Node>, Error> {
        log::trace!("parse_rule");

        let result = self.parse_rule_internal();
        if result.is_err() && self.config.ignore_errors {
            self.parse_until_rule_end();
            log::warn!("Ignoring error in parse_rule: {:?}", result);
            return Ok(None);
        }

        if let Ok(rule_node) = result {
            return Ok(Some(rule_node));
        }

        Ok(None)
    }

    fn parse_rule_internal(&mut self) -> Result<Node, Error> {
        let loc = self.tokenizer.current_location();

        let prelude = self.parse_selector_list()?;

        self.consume(TokenType::LCurly)?;
        self.consume_whitespace_comments();

        let block = self.parse_block(BlockParseMode::StyleBlock)?;

        self.consume(TokenType::RCurly)?;

        Ok(Node::new(
            NodeType::Rule {
                prelude: Some(prelude),
                block: Some(block),
            },
            loc,
        ))
    }
}

#[cfg(test)]
mod tests {
    use crate::walker::Walker;
    use gosub_shared::byte_stream::{ByteStream, Encoding};

    macro_rules! test {
        ($func:ident, $input:expr, $expected:expr) => {
            let mut stream = ByteStream::new(Encoding::UTF8, None);
            stream.read_from_str($input, Some(Encoding::UTF8));
            stream.close();

            let mut parser = crate::Css3::new(&mut stream);
            let result = parser.$func().unwrap().unwrap();

            let w = Walker::new(&result);
            assert_eq!(w.walk_to_string(), $expected);
        };
    }

    #[test]
    fn test_parse_rule() {
        test!(parse_rule, "body { color: red }", "[Rule]\n  [SelectorList (1)]\n    [Selector]\n      [Ident] body\n  [Block]\n    [Declaration] property: color important: false\n      [Ident] red\n");
        test!(
            parse_rule,
            "body { }",
            "[Rule]\n  [SelectorList (1)]\n    [Selector]\n      [Ident] body\n  [Block]\n"
        );
    }
}
