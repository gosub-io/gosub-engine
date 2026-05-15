use crate::types::{DocumentId, NavigationId, PrefetchId, TaskId};
use std::fmt::Display;

/// Request references indicate what initiated a request without the net layer needing to know
/// about higher-level engine concepts like tabs. This keeps the net module independent of the
/// engine's tab/navigation machinery while still allowing it to emit typed events.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Copy)]
pub enum RequestReference {
    /// Main doc for a tab
    Navigation(NavigationId),
    /// Sub resources of a specific doc
    Document(DocumentId),
    /// Background prefetches
    Prefetch(PrefetchId),
    /// Misc/system
    Background(TaskId),
}

impl Display for RequestReference {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RequestReference::Navigation(id) => write!(f, "Nav({})", id),
            RequestReference::Document(id) => write!(f, "Doc({})", id),
            RequestReference::Prefetch(id) => write!(f, "Prefetch({})", id),
            RequestReference::Background(id) => write!(f, "BG({})", id),
        }
    }
}
