mod handle;
pub mod history;
mod options;
pub(crate) mod remote_effects;
mod scroll;
pub mod services;
mod sink;
mod state;
#[allow(clippy::module_inception)]
mod tab;
mod worker;

pub use handle::TabHandle;
pub use tab::*;

pub use options::TabCookieJar;
pub(crate) use options::TabDefaults;
pub(crate) use options::TabOverrides;
pub use options::TabStorageScope;

pub use sink::TabSink;

pub use history::{HistoryEntryId, HistoryEntrySummary, HistorySnapshot};

// Tab management and tab-related types.
