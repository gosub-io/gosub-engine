use crate::node::{Node, NodeType};
use crate::tokenizer::TokenType;
use crate::Css3;
use gosub_shared::errors::CssError;
use gosub_shared::errors::CssResult;

impl Css3<'_> {
    pub fn parse_url(&mut self) -> CssResult<Node> {
        log::trace!("parse_url");

        let loc = self.tokenizer.current_location();

        let name = self.consume_function()?;
        if name.to_ascii_lowercase() != "url" {
            return Err(CssError::with_location(
                format!("Expected url, got {:?}", name).as_str(),
                self.tokenizer.current_location(),
            ));
        }

        let t = self.consume_any()?;
        let url = match t.token_type {
            TokenType::QuotedString(url) => url,
            _ => {
                return Err(CssError::with_location(
                    format!("Expected url, got {:?}", t).as_str(),
                    self.tokenizer.current_location(),
                ));
            }
        };

        self.consume(TokenType::RParen)?;

        Ok(Node::new(NodeType::Url { url }, loc))
    }
}

#[cfg(test)]
mod tests {
    use crate::walker::Walker;
    use crate::{CssOrigin, ParserConfig};
    use gosub_shared::byte_stream::{ByteStream, Encoding};

    macro_rules! test {
        ($func:ident, $input:expr, $expected:expr) => {
            let mut stream = ByteStream::new(Encoding::UTF8, None);
            stream.read_from_str($input, Some(Encoding::UTF8));
            stream.close();

            let mut parser = crate::Css3::new(&mut stream, ParserConfig::default(), CssOrigin::User, "");
            let result = parser.$func().unwrap();

            let w = Walker::new(&result);
            assert_eq!(w.walk_to_string(), $expected);
        };
    }

    // macro_rules! test_err {
    //     ($func:ident, $input:expr, $expected:expr) => {
    //         let mut stream = ByteStream::new(Encoding::UTF8, None);
    //         stream.read_from_str($input, Some(Encoding::UTF8));
    //         stream.close();
    //
    //         let mut parser = crate::Css3::new(&mut stream, Default::default(), Default::default(), "");
    //         let result = parser.$func();
    //
    //         assert_eq!(true, result.is_err());
    //         let err = result.unwrap_err();
    //
    //         assert_eq!(true, err.message.contains($expected));
    //     };
    // }

    #[test]
    fn test_parse_url() {
        test!(parse_url, "url(\"foobar\")", "[Url] foobar\n");
        test!(parse_url, "url(\'foobar\')", "[Url] foobar\n");
        test!(parse_url, "url(\"\")", "[Url] \n");
        // test_err!(parse_url, "url(\"\"]", "Expected RParen, got Token");
        // test_err!(parse_url, "url", "Expected function, got Token");
    }
}
