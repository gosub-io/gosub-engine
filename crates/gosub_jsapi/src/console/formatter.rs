use std::fmt;

/// Formatting structure
pub struct Formatter;

impl Formatter {
    /// Returns a new formatter
    #[must_use]
    pub fn new() -> Self {
        Self {}
    }

    /// Formats the given string based on the formatting arguments and data provided
    pub fn format(&self, args: &[&dyn fmt::Display]) -> String {
        let mut s = String::new();
        for arg in args {
            s.push_str(&format!("{arg} "));
        }

        s.trim_end().to_owned()
    }
}
