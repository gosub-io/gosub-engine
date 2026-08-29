//! Unix spawn backend: `fork` + `exec` via `std::process::Command`.

use std::io;

/// A spawned child process.
pub struct Child {
    inner: std::process::Child,
    /// What its profile asked for, for the parent-side cgroup placement.
    pub(crate) data_limit: u64,
    pub(crate) max_tasks: u32,
}

/// Environment the children keep. Everything else in the broker's environment
/// (proxy credentials, tokens, whatever the embedder's launcher set) stays
/// here; a compromised child reads its own `environ` regardless of `/proc`.
const ENV_KEPT: &[&str] = &[
    "HOME",
    "USER",
    "LOGNAME",
    "TMPDIR",
    "LANG",
    "LANGUAGE",
    "SSL_CERT_FILE",
    "SSL_CERT_DIR",
    "RUST_LOG",
    "RUST_BACKTRACE",
];
const ENV_KEPT_PREFIXES: &[&str] = &["LC_", "XDG_", "FONTCONFIG_", "GOSUB_"];

fn env_kept(key: &str) -> bool {
    ENV_KEPT.contains(&key) || ENV_KEPT_PREFIXES.iter().any(|p| key.starts_with(p))
}

impl Child {
    /// Wait for the child to exit, discarding its status.
    pub fn wait(&mut self) -> io::Result<()> {
        self.inner.wait().map(|_| ())
    }

    /// Wait, and describe how the child ended - an exit code, or the signal
    /// that killed it ("exited 1" vs "killed by signal 31 (SIGSYS)").
    pub fn wait_describe(&mut self) -> String {
        use std::os::unix::process::ExitStatusExt;
        match self.inner.wait() {
            Ok(status) => match (status.code(), status.signal()) {
                (Some(code), _) => format!("exited {code}"),
                (None, Some(sig)) => format!("killed by signal {sig}"),
                (None, None) => "ended for an unknown reason".to_string(),
            },
            Err(e) => format!("could not be reaped: {e}"),
        }
    }

    /// The child's process id - needed so the parent can place it in its own
    /// cgroup (the Linux half of `confine_spawned_child`).
    pub fn id(&self) -> u32 {
        self.inner.id()
    }

    /// Whether the child has exited, without blocking. `Ok(true)` reaps it.
    pub fn try_wait(&mut self) -> io::Result<bool> {
        self.inner.try_wait().map(|status| status.is_some())
    }

    /// Best-effort SIGKILL - used to abandon a child that has wedged (e.g. a
    /// decoder that stopped answering), so `wait` does not block forever.
    pub fn kill(&mut self) -> io::Result<()> {
        self.inner.kill()
    }
}

/// Spawn `exe` with `args`, handing `child_end` over as an inherited channel.
pub fn spawn(
    exe: &std::path::Path,
    args: &[&str],
    child_end: gosub_ipc::channel::Channel,
    isolation: crate::NamespaceIsolation,
    container: super::ContainerProfile<'_>,
) -> io::Result<Child> {
    use std::os::unix::process::CommandExt;

    let data_limit = container.data_limit.unwrap_or(crate::DEFAULT_CHILD_DATA_LIMIT);
    let file_size_limit = container.file_size_limit;
    let mut cmd = std::process::Command::new(exe);
    cmd.args(args).arg(child_end.to_argv());

    // An allowlisted environment. Among what this drops are the dynamic
    // loader's injection knobs (`LD_PRELOAD`, `DYLD_INSERT_LIBRARIES`), which
    // would run attacker code before the child's own lockdown.
    cmd.env_clear();
    for (key, value) in std::env::vars_os() {
        if key.to_str().is_some_and(env_kept) {
            cmd.env(&key, &value);
        }
    }
    // Nothing to read from a terminal, and no terminal `ioctl`s to reach.
    cmd.stdin(std::process::Stdio::null());

    let raw = child_end.raw();
    let extra_fds: Vec<i32> = container.extra_fds.to_vec();
    // SAFETY: the closure runs post-fork/pre-exec and calls only
    // async-signal-safe operations (setrlimit, setpriority, unshare, fcntl).
    unsafe {
        cmd.pre_exec(move || {
            crate::apply_child_rlimits_with(data_limit)?;
            if let Some(bytes) = file_size_limit {
                crate::apply_child_file_size_limit(bytes)?;
            }
            // Fail-closed, matching the seccomp precedent: a child that was
            // meant to be network-isolated and silently isn't is worse than an
            // honest refusal to start.
            crate::isolate_namespaces(isolation)?;
            // Every descriptor a C library left without CLOEXEC would otherwise
            // ride along; only the links named below survive the exec.
            crate::mark_all_fds_close_on_exec();
            gosub_ipc::channel::Channel::make_inheritable(raw)?;
            for fd in &extra_fds {
                gosub_ipc::channel::Channel::make_inheritable(*fd)?;
            }
            Ok(())
        });
    }

    let child = cmd.spawn().map_err(|e| {
        if e.raw_os_error() == Some(libc::EPERM) && isolation != crate::NamespaceIsolation::None {
            io::Error::new(
                e.kind(),
                format!(
                    "{e}; unprivileged user namespaces may be disabled on this host (see docs/process-isolation.md)"
                ),
            )
        } else {
            e
        }
    })?;
    // The child holds its own copy now; drop ours so a dead child is seen as
    // EOF rather than a link the engine is itself holding open.
    drop(child_end);
    Ok(Child {
        inner: child,
        data_limit,
        max_tasks: container.max_tasks,
    })
}
