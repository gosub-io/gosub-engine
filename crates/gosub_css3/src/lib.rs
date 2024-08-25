use crate::node::{Node, NodeType};
use crate::parser_config::{Context, ParserConfig};
use crate::tokenizer::Tokenizer;
use gosub_shared::byte_stream::{ByteStream, Encoding, Location};
use gosub_shared::{timing_start, timing_stop};

pub mod colors;
pub mod convert;
mod node;
pub mod parser;
pub mod parser_config;
pub mod stylesheet;
pub mod tokenizer;
mod unicode;
pub mod walker;

/// This CSS3 parser is heavily based on the MIT licensed CssTree parser written by
/// Roman Dvornov (https://github.com/lahmatiy).
/// The original version can be found at https://github.com/csstree/csstree

pub struct Css3<'stream> {
    /// The tokenizer is responsible for reading the input stream and
    pub tokenizer: Tokenizer<'stream>,
    /// When the last item is true, we allow values in argument lists.
    allow_values_in_argument_list: Vec<bool>,
    /// The parser configuration as given
    config: ParserConfig,
}

#[derive(Debug)]
pub struct Error {
    /// The error message
    pub message: String,
    /// The location of the error
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
        let t_id = timing_start!("css3.parse", config.source.as_deref().unwrap_or(""));

        let mut stream = ByteStream::new(Encoding::UTF8, None);
        stream.read_from_str(data, Some(Encoding::UTF8));
        stream.close();

        let mut parser = Css3::new(&mut stream);
        let result = parser.parse_internal(config);

        timing_stop!(t_id);

        match result {
            Ok(Some(node)) => Ok(node),
            Ok(None) => Ok(Node::new(
                NodeType::StyleSheet {
                    children: Vec::new(),
                },
                Location::default(),
            )),
            Err(e) => Err(e),
        }
    }

    /// Create a new parser with the given bytestream
    fn new(it: &'stream mut ByteStream) -> Self {
        Self {
            tokenizer: Tokenizer::new(it, Location::default()),
            allow_values_in_argument_list: Vec::new(),
            config: Default::default(),
        }
    }

    /// Actual parser implementation
    fn parse_internal(&mut self, config: ParserConfig) -> Result<Option<Node>, Error> {
        self.config = config;

        match self.config.context {
            Context::Stylesheet => self.parse_stylesheet(),
            Context::Rule => self.parse_rule(),
            Context::AtRule => self.parse_at_rule(true),
            Context::Declaration => self.parse_declaration(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::walker::Walker;
    use simple_logger::SimpleLogger;

    #[test]
    #[ignore]
    fn parser() {
        let filename = "../tests/data/css3-data/data.css";

        SimpleLogger::new().init().unwrap();

        let config = ParserConfig {
            source: Some(filename.to_string()),
            ignore_errors: true,
            ..Default::default()
        };

        let css = std::fs::read_to_string(filename).unwrap();
        let res = Css3::parse(css.as_str(), config);
        if res.is_err() {
            println!("{:?}", res.err().unwrap());
            return;
        }

        let binding = res.unwrap();
        let w = Walker::new(&binding);
        w.walk_stdout();
    }
}
