use crate::byte_stream::Location;

pub mod css3;
pub mod document;
pub mod html5;
pub mod node;

pub mod config;
pub mod draw;
pub mod render_tree;

/// Context defines how the data needs to be parsed
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Context {
    Stylesheet,
    Rule,
    AtRule,
    Declaration,
}

/// ParserConfig holds the configuration for the CSS3 parser
pub struct ParserConfig {
    /// Context defines what kind of data we are providing: a stylesheet, a rule, an at-rule or a declaration
    pub context: Context,
    /// Location holds the start position of the given element in the data source
    pub location: Location,
    /// Optional source filename or url
    pub source: Option<String>,
    /// Ignore errors and continue parsing. Any errors will not be returned in the final AST
    /// (this means if a selector is invalid, all rules will be ignored, even when they are valid)
    pub ignore_errors: bool,
    /// When true, the values in the declaration will be matched against the property syntax. If it doesn't
    /// match it will trigger an error
    pub match_values: bool,
}

impl Default for ParserConfig {
    fn default() -> Self {
        Self {
            context: Context::Stylesheet,
            location: Location::default(),
            source: None,
            ignore_errors: false,
            match_values: true,
        }
    }
}
