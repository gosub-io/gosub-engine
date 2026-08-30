//! Spawning a child component - the third platform seam, alongside
//! [`gosub_ipc::channel`] (how children talk) and this crate (how children
//! are confined).

/// Parent-supplied confinement profile for one child, applied at create time on
/// Windows (an AppContainer identity plus the path grants that container needs)
/// and ignored on Unix, where a child confines itself after `exec`.
///
/// The caller names the container; keep the name distinct per role, so a grant
/// to one container never widens another role's reach. `fs_grant` is the path
/// a role needs - the Windows analogue of `openat` plus Landlock.
#[derive(Clone, Copy, Debug, Default)]
pub struct ContainerProfile<'a> {
    pub name: &'a str,
    /// Grant the `internetClient` capability - the network role only.
    pub internet: bool,
    /// A path this role's container may reach, with a writable flag.
    pub fs_grant: Option<(&'a std::path::Path, bool)>,
    /// Committed-memory ceiling for this role, in bytes; `None` is
    /// [`DEFAULT_CHILD_DATA_LIMIT`](crate::DEFAULT_CHILD_DATA_LIMIT). The
    /// renderer family is the one role that legitimately holds a lot.
    pub data_limit: Option<u64>,
    /// Further descriptors the child inherits besides its primary link (a
    /// second channel, named in `args`). Made inheritable only in the child,
    /// after the fork, so no other child spawned meanwhile can receive them.
    /// Unix only.
    pub extra_fds: &'a [i32],
    /// Tasks (processes and threads) this role may have at once - the
    /// fork-bomb bound, enforced by the child's cgroup where one exists.
    pub max_tasks: u32,
    /// Largest file the role may write (`RLIMIT_FSIZE`); `None` leaves the
    /// inherited limit. Only the storage service writes files at all.
    pub file_size_limit: Option<u64>,
}

#[cfg(unix)]
mod unix;
#[cfg(unix)]
pub use unix::{spawn, Child};

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::{spawn, Child};
