use std::fmt::{self, Write};

/// Formatting structure
pub struct Formatter;

impl Formatter {
    /// Formats the given string based on the formatting arguments and data provided
    pub fn format(args: &[&dyn fmt::Display]) -> String {
        let mut s = String::new();
        for arg in args {
            let _ = write!(s, "{arg} ");
        }

        s.trim_end().to_owned()
    }
}
