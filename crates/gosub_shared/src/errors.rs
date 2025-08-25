use crate::byte_stream::Location;
use std::fmt::{Display, Formatter};

/// Parser error that defines an error (message) on the given position
#[derive(Clone, Debug, PartialEq)]
pub struct CssError {
    /// Error message
    pub message: String,
    /// Location of the error, if available (during parsing mostly)
    pub location: Option<Location>,
}

impl CssError {
    #[must_use]
    pub fn new(message: &str) -> Self {
        CssError {
            message: message.to_string(),
            location: None,
        }
    }

    #[must_use]
    pub fn with_location(message: &str, location: Location) -> Self {
        CssError {
            message: message.to_string(),
            location: Some(location),
        }
    }
}

impl Display for CssError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        if self.location.is_some() {
            write!(f, "{} at {:?}", self.message, self.location)
        } else {
            write!(f, "{}", self.message)
        }
    }
}

pub type CssResult<T> = Result<T, CssError>;
