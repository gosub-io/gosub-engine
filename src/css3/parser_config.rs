use crate::css3::location::Location;

/// Context defines how the data needs to be parsed
pub enum Context {
    Stylesheet,
    Rule,
    AtRule,
    Declaration,
}

/// ParserConfig holds the configuration for the parser
pub struct ParserConfig {
    /// Context defines how the data needs to be parsed
    pub context: Context,
    /// Location holds the start position of the given element in the data source
    pub location: Location,
    /// Optional source filename or url
    pub source: Option<String>,
}

impl Default for ParserConfig {
    fn default() -> Self {
        Self {
            context: Context::Stylesheet,
            location: Location::default(),
            source: None,
        }
    }
}
