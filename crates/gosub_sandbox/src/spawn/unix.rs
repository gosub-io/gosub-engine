//! Unix spawn backend: `fork` + `exec` via `std::process::Command`.

use std::io;

/// A spawned child process.
pub struct Child(std::process::Child);

impl Child {
    /// Wait for the child to exit, discarding its status.
    pub fn wait(&mut self) -> io::Result<()> {
        self.0.wait().map(|_| ())
    }

    /// Wait, and describe how the child ended - an exit code, or the signal
    /// that killed it ("exited 1" vs "killed by signal 31 (SIGSYS)").
    pub fn wait_describe(&mut self) -> String {
        use std::os::unix::process::ExitStatusExt;
        match self.0.wait() {
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
        self.0.id()
    }

    /// Whether the child has exited, without blocking. `Ok(true)` reaps it.
    pub fn try_wait(&mut self) -> io::Result<bool> {
        self.0.try_wait().map(|status| status.is_some())
    }

    /// Best-effort SIGKILL - used to abandon a child that has wedged (e.g. a
    /// decoder that stopped answering), so `wait` does not block forever.
    pub fn kill(&mut self) -> io::Result<()> {
        self.0.kill()
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
    let mut cmd = std::process::Command::new(exe);
    cmd.args(args).arg(child_end.to_argv());

    // Strip the dynamic-loader injection vectors from the child's environment
    // before it execs: DYLD_INSERT_LIBRARIES (macOS) and LD_PRELOAD/LD_* (glibc)
    // are the runtime linker's "load this code into every new process" knobs. A
    // child inheriting one would run attacker-supplied library code *before* it
    // reaches its own lockdown, sidestepping the sandbox entirely.
    for (key, _) in std::env::vars_os() {
        if key
            .to_str()
            .is_some_and(|k| k.starts_with("DYLD_") || k.starts_with("LD_"))
        {
            cmd.env_remove(&key);
        }
    }

    let raw = child_end.raw();
    // SAFETY: the closure runs post-fork/pre-exec and calls only
    // async-signal-safe operations (setrlimit, setpriority, unshare, fcntl).
    unsafe {
        cmd.pre_exec(move || {
            crate::apply_child_rlimits_with(data_limit)?;
            // Fail-closed, matching the seccomp precedent: a child that was
            // meant to be network-isolated and silently isn't is worse than an
            // honest refusal to start.
            crate::isolate_namespaces(isolation)?;
            gosub_ipc::channel::Channel::make_inheritable(raw)?;
            Ok(())
        });
    }

    let child = cmd.spawn()?;
    // The child holds its own copy now; drop ours so a dead child is seen as
    // EOF rather than a link the engine is itself holding open.
    drop(child_end);
    Ok(Child(child))
}
