use crate::css3::node::{Node, NodeType};
use crate::css3::tokenizer::TokenType;
use crate::css3::{Css3, Error};

impl Css3<'_> {
    pub fn parse_url(&mut self) -> Result<Node, Error> {
        log::trace!("parse_url");

        let loc = self.tokenizer.current_location().clone();

        let name = self.consume_function()?;
        if name.to_ascii_lowercase() != "url" {
            return Err(Error::new(
                format!("Expected url, got {:?}", name),
                self.tokenizer.current_location().clone(),
            ));
        }

        let t = self.consume_any()?;
        let url = match t.token_type {
            TokenType::QuotedString(url) => url,
            _ => {
                return Err(Error::new(
                    format!("Expected url, got {:?}", t),
                    self.tokenizer.current_location().clone(),
                ))
            }
        };

        self.consume(TokenType::RParen)?;

        Ok(Node::new(NodeType::Url { url }, loc))
    }
}

#[cfg(test)]
mod tests {
    use crate::byte_stream::Stream;
    use crate::css3::walker::Walker;

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

    macro_rules! test_err {
        ($func:ident, $input:expr, $expected:expr) => {
            let mut it = crate::css3::ByteStream::new();
            it.read_from_str($input, Some(crate::byte_stream::Encoding::UTF8));
            it.close();

            let mut parser = crate::css3::Css3::new(&mut it);
            let result = parser.$func();

            assert_eq!(true, result.is_err());
            let err = result.unwrap_err();

            assert_eq!(true, err.message.contains($expected));
        };
    }

    #[test]
    fn test_parse_url() {
        test!(parse_url, "url(\"foobar\")", "[Url] foobar\n");
        test!(parse_url, "url(\'foobar\')", "[Url] foobar\n");
        test!(parse_url, "url(\"\")", "[Url] \n");
        test_err!(parse_url, "url(\"\"]", "Expected RParen, got Token");
        test_err!(parse_url, "url", "Expected function, got Token");
    }
}
