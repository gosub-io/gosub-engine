use std::fmt;

pub struct Formatter {}

impl Formatter {
    pub fn new() -> Formatter {
        Formatter {}
    }

    pub fn format(&self, args: &[&dyn fmt::Display]) -> String {
        let mut s = String::from("");
        for arg in args {
            s.push_str(format!("{} ", arg).as_str());
        }

        s.trim_end().to_string()
    }
}
