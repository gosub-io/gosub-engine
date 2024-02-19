use crate::node::{Node, NodeType};
use crate::parser::block::BlockParseMode;
use crate::tokenizer::TokenType;
use crate::{Css3, Error};

impl Css3<'_> {
    pub fn parse_rule(&mut self) -> Result<Node, Error> {
        log::trace!("parse_rule");
        let loc = self.tokenizer.current_location().clone();

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
    use crate::byte_stream::Stream;
    use crate::walker::Walker;

    macro_rules! test {
        ($func:ident, $input:expr, $expected:expr) => {
            let mut it = crate::css3::ByteStream::new();
            it.read_from_str($input, Some(crate::byte_stream::Encoding::UTF8));
            it.close();

            let mut parser = crate::css3::Css3::new(&mut it);
            let result = parser.$func().unwrap();

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
