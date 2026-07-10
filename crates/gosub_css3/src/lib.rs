extern crate core;

use crate::ast::convert_ast_to_stylesheet;
use crate::stylesheet::CssStylesheet;
use crate::tokenizer::Tokenizer;

use gosub_interface::css3::CssOrigin;
use gosub_shared::byte_stream::{ByteStream, Encoding, Location};
use gosub_shared::config::Context;
use gosub_shared::config::ParserConfig;
use gosub_shared::errors::{CssError, CssResult};
use gosub_shared::{timing_start, timing_stop};

pub mod ast;
/// This CSS3 parser is heavily based on the MIT licensed `CssTree` parser written by
/// Roman Dvornov (<https://github.com/lahmatiy>).
/// The original version can be found at <https://github.com/csstree/csstree>
pub mod colors;
pub mod errors;
mod functions;
#[allow(dead_code)]
pub mod matcher;
// The as_* accessors panic by contract when called on the wrong node type;
// callers are expected to check the matching is_* predicate first.
#[allow(clippy::panic)]
pub mod node;
pub mod parser;
pub mod stylesheet;
pub mod system;
pub mod tokenizer;
mod unicode;
pub mod walker;

/// Cap on recursive-descent depth, shared by every recursive cycle in the parser.
///
/// Each level costs a stack frame, so unbounded input (`@media screen{` or `:is(` repeated) would
/// overflow the stack and abort the process -- an abort no `Result` or `catch_unwind` can
/// intercept. A debug frame measures ~9 KiB and a 2 MiB stack (what the threads we parse on get)
/// overflows between 192 and 256 levels, so the cap has to leave room for a stack the callers above
/// us have already partly consumed. 64 levels is far beyond the handful real stylesheets nest.
const MAX_RECURSION_DEPTH: usize = 64;

pub struct Css3<'stream> {
    /// The tokenizer is responsible for reading the input stream and
    pub tokenizer: Tokenizer<'stream>,
    /// When true, we allow values in argument lists.
    allow_values_in_argument_list: Vec<bool>,
    /// The parser configuration as given
    config: ParserConfig,
    /// Origin of the stream (useragent, inline etc.)
    origin: CssOrigin,
    /// Source of the stream (filename, url, etc.)
    source: String,
    /// Current recursive-descent depth; capped to prevent stack overflow on adversarial input.
    recursion_depth: usize,
}

impl<'stream> Css3<'stream> {
    /// Creates a new parser with the given byte stream so only `parse()` needs to be called.
    fn new(stream: &'stream mut ByteStream, config: ParserConfig, origin: CssOrigin, source: &str) -> Self {
        Self {
            tokenizer: Tokenizer::new(stream, Location::default()),
            allow_values_in_argument_list: Vec::new(),
            config,
            origin,
            source: source.to_string(),
            recursion_depth: 0,
        }
    }

    /// Runs `f` one level deeper, refusing to descend past [`MAX_RECURSION_DEPTH`].
    ///
    /// Every recursive cycle in the parser (blocks, functions, `calc()` parentheses, selector
    /// lists inside `:is()` and friends) routes through here, so a document that mixes them cannot
    /// sum their individual depths into an overflow.
    fn recurse<T>(&mut self, f: impl FnOnce(&mut Self) -> CssResult<T>) -> CssResult<T> {
        if self.recursion_depth >= MAX_RECURSION_DEPTH {
            return Err(CssError::with_location(
                "nesting too deep",
                self.tokenizer.current_location(),
            ));
        }

        self.recursion_depth += 1;
        let result = f(self);
        self.recursion_depth -= 1;

        result
    }

    /// Parses a direct string to a `CssStyleSheet`
    pub fn parse_str(
        data: &str,
        config: ParserConfig,
        origin: CssOrigin,
        source_url: &str,
    ) -> CssResult<CssStylesheet> {
        let mut stream = ByteStream::from_str(data, Encoding::UTF8);

        Css3::parse_stream(&mut stream, config, origin, source_url)
    }

    /// Parses a direct stream to a `CssStyleSheet`
    pub fn parse_stream(
        stream: &mut ByteStream,
        config: ParserConfig,
        origin: CssOrigin,
        source_url: &str,
    ) -> CssResult<CssStylesheet> {
        Css3::new(stream, config, origin, source_url).parse()
    }

    fn parse(&mut self) -> CssResult<CssStylesheet> {
        if self.config.context != Context::Stylesheet {
            return Err(CssError::new("Expected a stylesheet context"));
        }

        let t_id = timing_start!("css3.parse", self.config.source.as_deref().unwrap_or(""));

        // let mut stream = ByteStream::new(Encoding::UTF8, None);
        // stream.read_from_str(data, Some(Encoding::UTF8));
        // stream.close();

        let node_tree = match self.config.context {
            Context::Stylesheet => self.parse_stylesheet_internal(),
            Context::Rule => self.parse_rule(),
            Context::AtRule => self.parse_at_rule(true),
            Context::Declaration => self.parse_declaration(),
        };

        timing_stop!(t_id);

        match node_tree {
            Ok(None) => Err(CssError::new("No node tree found")),
            Ok(Some(node)) => convert_ast_to_stylesheet(&node, self.origin, self.source.clone().as_str()),
            Err(e) => Err(e),
        }
    }
}

/// Loads the default user agent stylesheet
#[must_use]
pub fn load_default_useragent_stylesheet() -> CssStylesheet {
    // @todo: we should be able to browse to gosub:useragent.css and see the actual useragent css file
    let url = "gosub:useragent.css";

    let config = ParserConfig {
        ignore_errors: true,
        match_values: true,
        ..Default::default()
    };

    let css_data = include_str!("../resources/useragent.css");
    #[allow(clippy::expect_used)] // PANIC-SAFE: compiled-in stylesheet, exercised by every parser test
    Css3::parse_str(css_data, config, CssOrigin::UserAgent, url).expect("Could not parse useragent stylesheet")
}

#[cfg(test)]
mod tests {
    use super::*;
    // use crate::walker::Walker;
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
        let res = Css3::parse_str(css.as_str(), config, CssOrigin::Author, filename);
        if res.is_err() {
            println!("{:?}", res.err().unwrap());
        }

        // let binding = res.unwrap();
        // let w = Walker::new(&binding);
        // w.walk_stdout();
    }
}
