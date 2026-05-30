use std::fmt::Display;
use uuid::Uuid;

/// A unique identifier for a browser tab.
///
/// Internally a [`Uuid`] wrapper. Treat it as an opaque handle.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TabId(Uuid);

impl Default for TabId {
    fn default() -> Self {
        Self::new()
    }
}

impl TabId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Display for TabId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
