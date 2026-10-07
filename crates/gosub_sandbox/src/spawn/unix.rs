//! Unix spawn backend: `fork` + `exec` via `std::process::Command`.

use std::io;

/// A spawned child process.
pub struct Child {
    inner: std::process::Child,
    /// What its profile asked for, for the parent-side cgroup placement
    /// (cgroups are Linux's; macOS carries these unread).
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pub(crate) data_limit: u64,
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
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
/// `GOSUB_DUMP_*` name files for the layout and style dumps to write during
/// a render. A confined renderer has no `openat`: the dump would be a SIGSYS
/// on every page while the variable is set, so it stays with the broker.
const ENV_DROPPED_PREFIXES: &[&str] = &["GOSUB_DUMP_"];

/// Whether a child keeps `key`. Also what the escape audit holds a child's
/// environment against.
pub(crate) fn env_kept(key: &str) -> bool {
    if ENV_DROPPED_PREFIXES.iter().any(|p| key.starts_with(p)) {
        return false;
    }
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
    // No terminal at all. A broker started from a shell would otherwise hand
    // every child the tty read-write on all three: keystrokes to read while
    // the browser is the foreground job, escape sequences to write, and on
    // kernels with legacy `TIOCSTI` input to queue into the user's shell.
    // Stdout has nothing to say; stderr carries the lockdown banners and
    // crash reports, so it comes back through a pipe this process relays.
    cmd.stdin(std::process::Stdio::null());
    cmd.stdout(std::process::Stdio::null());
    cmd.stderr(std::process::Stdio::piped());

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
    let mut child = child;
    if let Some(stderr) = child.stderr.take() {
        relay_stderr(stderr, child.id());
    }
    Ok(Child {
        inner: child,
        data_limit,
        max_tasks: container.max_tasks,
    })
}

/// The longest line of a child's stderr passed on; the rest of it is dropped.
const MAX_RELAYED_LINE: usize = 4096;

/// Pass a child's stderr on to this process's own, a line at a time: cut to
/// [`MAX_RELAYED_LINE`], and with control characters other than tab removed, so
/// whatever a compromised child writes reaches a terminal as text and never as
/// an escape sequence. Each line starts with `[child <pid>]`, so nothing a child
/// writes passes for the broker's own diagnostics. Runs until every holder of
/// the pipe's write end - the child and anything it forked - has exited.
fn relay_stderr(stderr: std::process::ChildStderr, pid: u32) {
    let spawned = std::thread::Builder::new()
        .name("gosub-child-stderr".into())
        .spawn(move || {
            use std::io::{BufRead, Read, Write};
            let mut reader = io::BufReader::new(stderr);
            let mut line = Vec::new();
            loop {
                line.clear();
                // Bounded read: at most one line, and at most the cap plus one
                // byte of it, so a child that never writes a newline costs
                // nothing but the cap.
                let (read, failed) = match (&mut reader)
                    .take(MAX_RELAYED_LINE as u64 + 1)
                    .read_until(b'\n', &mut line)
                {
                    Ok(0) => return,
                    Ok(n) => (n, false),
                    Err(_) if line.is_empty() => return,
                    // A read error ends the relay, but what arrived before it
                    // still goes out.
                    Err(_) => (line.len(), true),
                };
                let ended = line.last() == Some(&b'\n');
                if !ended && !failed && read > MAX_RELAYED_LINE {
                    // Skip the rest of an over-long line.
                    let mut rest = Vec::new();
                    loop {
                        rest.clear();
                        match (&mut reader).take(64 * 1024).read_until(b'\n', &mut rest) {
                            Ok(0) | Err(_) => break,
                            Ok(_) if rest.last() == Some(&b'\n') => break,
                            Ok(_) => {}
                        }
                    }
                }
                let text = sanitize_line(&line);
                let _ = writeln!(io::stderr().lock(), "[child {pid}] {text}");
                if failed {
                    return;
                }
            }
        });
    if let Err(e) = spawned {
        // Without a reader the child blocks once the pipe fills; its stderr is
        // diagnostics, not worth that.
        eprintln!("[gosub] could not start the relay for a child's stderr: {e}");
    }
}

/// One relayed line as text: lossy UTF-8, at most [`MAX_RELAYED_LINE`] bytes of
/// it, with control characters other than tab removed.
fn sanitize_line(line: &[u8]) -> String {
    let cut = &line[..line.len().min(MAX_RELAYED_LINE)];
    String::from_utf8_lossy(cut)
        .chars()
        .filter(|&c| c == '\t' || !c.is_control())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{sanitize_line, MAX_RELAYED_LINE};

    #[test]
    fn a_relayed_line_carries_no_escape_sequences() {
        assert_eq!(
            sanitize_line(b"\x1b[2J\x1b]0;owned\x07[net] ok\tdone\r\n"),
            "[2J]0;owned[net] ok\tdone"
        );
    }

    #[test]
    fn a_relayed_line_is_bounded() {
        let long = vec![b'a'; MAX_RELAYED_LINE * 3];
        assert_eq!(sanitize_line(&long).len(), MAX_RELAYED_LINE);
    }
}
