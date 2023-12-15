use crate::byte_stream::{ByteStream, Encoding, Stream};
use crate::css3::location::Location;
use crate::css3::node::Node;
use crate::css3::parser_config::{Context, ParserConfig};
use crate::css3::tokenizer::Tokenizer;

mod location;
mod node;
mod parser;
pub mod parser_config;
mod tokenizer;
mod unicode;
pub mod walker;

/// This CSS3 parser is heavily based on the MIT licensed CssTree parser written by
/// Roman Dvornov (https://github.com/lahmatiy).
/// The original version can be found at https://github.com/csstree/csstree

pub struct Css3<'stream> {
    pub tokenizer: Tokenizer<'stream>,
}

#[derive(Debug)]
pub struct Error {
    pub message: String,
    pub location: Location,
}

impl Error {
    pub(crate) fn new(message: String, location: Location) -> Error {
        Error { message, location }
    }
}

impl<'stream> Css3<'stream> {
    /// Parse a CSS string, which depends on the context.
    pub fn parse(data: &str, config: ParserConfig) -> Result<Node, Error> {
        let mut it = ByteStream::new();
        it.read_from_str(data, Some(Encoding::UTF8));
        it.close();

        let mut parser = Css3::new(&mut it);
        parser.parse_internal(config)
    }

    fn new(it: &'stream mut ByteStream) -> Self {
        Self {
            tokenizer: Tokenizer::new(it, Location::default()),
        }
    }

    fn parse_internal(&mut self, config: ParserConfig) -> Result<Node, Error> {
        match config.context {
            Context::Stylesheet => self.parse_stylesheet(),
            Context::Rule => self.parse_rule(),
            Context::AtRule => self.parse_at_rule(true),
            Context::Declaration => self.parse_declaration(),
        }
    }
}

#[cfg(test)]
mod tests {
    use simple_logger::SimpleLogger;
    use super::*;

    #[test]
    fn parser() {
        let filename = "ms2.css";

        SimpleLogger::new().init().unwrap();

        let config = ParserConfig {
            source: Some(filename.to_string()),
            ..Default::default()
        };

        let css = std::fs::read_to_string(filename).unwrap();
        let res = Css3::parse(css.as_str(), config);
        if res.is_err() {
            println!("{:?}", res.err().unwrap());
            return;
        }

        walker::Walker.walk(&res.unwrap());
    }
}
