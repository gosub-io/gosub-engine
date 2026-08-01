//! Spawning a child component — the third platform seam, alongside
//! [`gosub_ipc::channel`] (how children talk) and this crate (how children
//! are confined).
//!
//! ## Why this exists rather than `std::process::Command`
//!
//! ## What the Windows path buys immediately

/// Parent-supplied confinement profile for one child, applied at create time on
/// Windows (an AppContainer identity plus the path grants that container needs)
/// and ignored on Unix, where a child confines itself after `exec`.
///
/// The caller names the container because this crate has no notion of engine
/// roles. Keep the name **distinct per role**: a grant to one container must
/// never widen another role's reach. `fs_grant` is the path a role legitimately
/// needs — the Windows analogue of a Linux service's `openat` plus Landlock.
#[derive(Clone, Copy, Debug, Default)]
pub struct ContainerProfile<'a> {
    pub name: &'a str,
    /// Grant the `internetClient` capability — the network role only.
    pub internet: bool,
    /// A path this role's container may reach, with a writable flag.
    pub fs_grant: Option<(&'a std::path::Path, bool)>,
}

#[cfg(unix)]
mod unix;
#[cfg(unix)]
pub use unix::{spawn, Child};

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::{spawn, Child};
