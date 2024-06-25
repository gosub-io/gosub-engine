use gosub_shared::byte_stream::Location;

/// Context defines how the data needs to be parsed
pub enum Context {
    Stylesheet,
    Rule,
    AtRule,
    Declaration,
}

/// ParserConfig holds the configuration for the CSS3 parser
pub struct ParserConfig {
    /// Context defines how the data needs to be parsed
    pub context: Context,
    /// Location holds the start position of the given element in the data source
    pub location: Location,
    /// Optional source filename or url
    pub source: Option<String>,
    /// Ignore errors and continue parsing. Any errors will not be returned in the final AST
    /// (this means if a selector is invalid, all rules will be ignored, even when they are valid)
    pub ignore_errors: bool,
}

impl Default for ParserConfig {
    fn default() -> Self {
        Self {
            context: Context::Stylesheet,
            location: Location::default(),
            source: None,
            ignore_errors: false,
        }
    }
}
