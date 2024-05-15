use std::fmt::{self, Write as _};

/// Formatting structure
pub struct Formatter;

impl Formatter {
    /// Returns a new formatter
    #[must_use]
    pub const fn new() -> Self {
        Self {}
    }

    /// Formats the given string based on the formatting arguments and data provided
    #[allow(clippy::unused_self)]
    pub fn format(&self, args: &[&dyn fmt::Display]) -> String {
        let mut s = String::new();
        for arg in args {
            write!(s, "{arg}").unwrap(); // unreachable
        }

        s.trim_end().to_owned()
    }
}
