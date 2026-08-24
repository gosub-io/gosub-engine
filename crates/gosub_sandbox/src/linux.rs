//! Linux backend: seccomp-BPF confinement, network namespaces, rlimits, and
//! `prctl(PR_SET_DUMPABLE)`. Only compiled on `target_os = "linux"`, so items
//! carry no `target_os` guards. Each child installs a seccomp-BPF filter after
//! connecting its IPC link (the same mechanism Chromium uses for renderers).

// Syscall numbers are `c_long` (i64 on 64-bit, i32 on 32-bit). The casts to
// i64 (seccompiler's rule key) are redundant only on 64-bit; dropping them
// breaks 32-bit builds.
#![allow(clippy::unnecessary_cast)]

/// Syscalls any confined child needs after startup: I/O on already-open fds
/// (its IPC socket + stderr), memory management, synchronization, signals,
/// time, teardown. Deliberately ABSENT: `socket`/`connect` (no new network),
/// `openat` (no file opens), `execve`/`clone` (no new programs/processes),
/// `io_uring_*` (no async-submission network bypass), `ptrace`.
#[cfg(feature = "multi-process")]
const BASELINE: &[libc::c_long] = &[
    // I/O on existing fds only - a new socket/file fd cannot be obtained
    // because socket()/openat() are not on this list.
    libc::SYS_read,
    libc::SYS_write,
    libc::SYS_readv,
    libc::SYS_writev,
    libc::SYS_recvfrom,
    libc::SYS_sendto,
    libc::SYS_recvmsg,
    libc::SYS_sendmsg,
    libc::SYS_close,
    // Both spellings of fstat: glibc 2.36 (Debian bookworm) issues
    // `newfstatat` with AT_EMPTY_PATH, 2.39 (Ubuntu 24.04) issues `fstat`.
    // Allowing only one kills the ring and tile consumers on the other libc.
    libc::SYS_fstat,
    libc::SYS_newfstatat,
    libc::SYS_statx,
    libc::SYS_lseek,
    // memory - mmap/mprotect are argument-filtered in `install` to forbid
    // PROT_EXEC (mremap preserves an existing mapping's protection, so it can't
    // introduce exec).
    libc::SYS_mmap,
    libc::SYS_munmap,
    libc::SYS_mremap,
    libc::SYS_mprotect,
    libc::SYS_madvise,
    libc::SYS_brk,
    // shared-memory tile transport: create an anonymous sealable buffer, size
    // it, seal it. fcntl is argument-filtered in `install` to the two seal
    // commands only - its other commands (F_DUPFD, F_SETFD/F_SETFL, locks)
    // stay fatal. memfd_create yields a plain memory fd: it opens no path on
    // the filesystem, so this adds no reach `openat`'s absence was denying.
    libc::SYS_memfd_create,
    libc::SYS_ftruncate,
    libc::SYS_fcntl,
    // runtime / synchronization
    libc::SYS_futex,
    // glibc registers an rseq area per thread at thread startup. A thread
    // spawned after lockdown (tokio grows its blocking pool lazily; DNS lands
    // on one) otherwise dies silently: glibc blocks signals during thread
    // setup, so the SIGSYS kills the process before the reporter runs.
    // Grants no reach; Chromium's baseline allows it too.
    libc::SYS_rseq,
    libc::SYS_getrandom,
    libc::SYS_sched_yield,
    libc::SYS_sched_getaffinity,
    libc::SYS_membarrier,
    // signals (Rust installs runtime handlers)
    libc::SYS_rt_sigreturn,
    libc::SYS_rt_sigprocmask,
    libc::SYS_rt_sigaction,
    libc::SYS_sigaltstack,
    // time
    libc::SYS_clock_gettime,
    libc::SYS_clock_nanosleep,
    libc::SYS_nanosleep,
    libc::SYS_gettimeofday,
    // identity (cheap, non-escalating)
    libc::SYS_getpid,
    libc::SYS_gettid,
    // teardown
    libc::SYS_exit,
    libc::SYS_exit_group,
];

/// The network syscalls the net component additionally needs. A real net
/// daemon would also need `openat` (resolv.conf/hosts) and DNS plumbing; the
/// PoC synthesizes responses so the socket family alone models the intent.
#[cfg(feature = "multi-process")]
const NET_EXTRA: &[libc::c_long] = &[
    libc::SYS_socket,
    libc::SYS_socketpair,
    libc::SYS_connect,
    libc::SYS_getsockopt,
    libc::SYS_setsockopt,
    libc::SYS_getsockname,
    libc::SYS_getpeername,
];

/// What a real network stack needs on top of [`NET_EXTRA`]. Every entry was
/// added because a request died on it (the SIGSYS handler names the call);
/// expected to grow as more libc/TLS-backend combinations are exercised.
#[cfg(feature = "multi-process")]
const NET_RUNTIME_EXTRA: &[libc::c_long] = &[
    // Async I/O readiness: tokio's reactor.
    libc::SYS_epoll_create1,
    libc::SYS_epoll_ctl,
    libc::SYS_epoll_wait,
    libc::SYS_epoll_pwait,
    libc::SYS_eventfd2,
    libc::SYS_poll,
    libc::SYS_ppoll,
    // Blocking-pool threads: DNS resolution runs on one.
    libc::SYS_clone,
    libc::SYS_clone3,
    libc::SYS_set_robust_list,
    libc::SYS_rt_sigtimedwait,
    libc::SYS_tgkill,
    // Enumerating the trust store directory, and following its symlinks.
    libc::SYS_getdents64,
    libc::SYS_readlinkat,
    libc::SYS_readlink,
    // glibc's resolver sends the A and AAAA queries in one `sendmmsg` and
    // collects them with `recvmmsg` - batched spellings of the single-datagram
    // calls above, no extra reach.
    libc::SYS_sendmmsg,
    libc::SYS_recvmmsg,
    // Socket options and non-blocking flags the client sets per connection.
    libc::SYS_ioctl,
    libc::SYS_shutdown,
    libc::SYS_bind,
    libc::SYS_getpeername,
    // Host and limit queries made while the stack initialises.
    libc::SYS_uname,
    libc::SYS_prlimit64,
    libc::SYS_getuid,
    libc::SYS_geteuid,
    libc::SYS_getgid,
    libc::SYS_getegid,
];

/// Extra syscalls the fork server needs on top of the baseline: making
/// renderers, and reaping them.
#[cfg(feature = "multi-process")]
const FORK_SERVER_EXTRA: &[libc::c_long] = &[
    // All three spellings of "make a process": glibc issues `clone3` (new) or
    // `clone` (older), musl issues legacy `SYS_fork`. Allowing only the glibc
    // pair kills the fork server on musl.
    #[cfg(any(target_arch = "x86_64", target_arch = "x86"))]
    libc::SYS_fork,
    libc::SYS_clone,
    libc::SYS_clone3,
    libc::SYS_wait4,
    libc::SYS_prctl,
    libc::SYS_seccomp,
    // Libc post-fork housekeeping in the child, before our code runs: glibc
    // resets the robust-futex list, musl registers a TID address. Both only
    // register a pointer for the kernel to clear on exit; neither escalates.
    libc::SYS_set_robust_list,
    libc::SYS_set_tid_address,
    // The fork server creates each renderer's private link after its own
    // lockdown, one pair per fork. A socketpair reaches no network.
    libc::SYS_socketpair,
];

/// Prove, on *this* machine, that the fork-server filter actually permits what
/// a forked renderer needs - before any renderer depends on it.
#[cfg(feature = "multi-process")]
pub fn verify_fork_server_filter() {
    // SAFETY: the fork server is single-threaded, so the child may run normal
    // code (the async-signal-safe-only rule applies to multithreaded fork).
    let pid = unsafe { libc::fork() };
    if pid < 0 {
        fail_canary(&format!("could not fork: {}", std::io::Error::last_os_error()));
    }

    if pid == 0 {
        // The child: do what a renderer does before it is confined.
        // SAFETY: fd 2 is open; F_DUPFD_CLOEXEC returns a new descriptor.
        let duped = unsafe { libc::fcntl(2, libc::F_DUPFD_CLOEXEC, 0) };
        if duped < 0 {
            unsafe { libc::_exit(EXIT_CANARY_DUP) };
        }
        unsafe { libc::close(duped) };
        // Installing a filter is `prctl` + `seccomp`, both issued *under* the
        // filter we inherited. Silent: `enforce` would print a second lockdown
        // banner and this is a probe, not a component starting up.
        if install(BASELINE.to_vec()).is_err() {
            unsafe { libc::_exit(EXIT_CANARY_SECCOMP) };
        }
        unsafe { libc::_exit(0) };
    }

    let mut status: libc::c_int = 0;
    // SAFETY: `status` is a valid out-param for our own child.
    if unsafe { libc::waitpid(pid, &mut status, 0) } != pid {
        fail_canary("could not reap the canary child (is wait4 allowed?)");
    }

    if libc::WIFSIGNALED(status) {
        let sig = libc::WTERMSIG(status);
        fail_canary(&format!(
            "canary child killed by signal {sig}{} — the allowlist is missing a \
             syscall this C library needs (glibc issues clone3 on newer versions, \
             clone on older; musl differs again)",
            if sig == libc::SIGSYS { " (SIGSYS)" } else { "" }
        ));
    }
    match libc::WEXITSTATUS(status) {
        0 => {}
        EXIT_CANARY_DUP => fail_canary(
            "fcntl(F_DUPFD_CLOEXEC) refused — a forked \
             renderer could not split its endpoint",
        ),
        EXIT_CANARY_SECCOMP => fail_canary(
            "the child could not install its own \
             seccomp filter — are prctl and seccomp on the allowlist?",
        ),
        other => fail_canary(&format!("canary child exited {other}")),
    }
}

/// Install a deliberately incomplete fork-server filter and run the canary
/// against it, so the *detection* is tested and not merely the happy path.
#[cfg(feature = "multi-process")]
pub fn canary_must_detect_a_missing_syscall() -> ! {
    // The deliberate gap is the `F_DUPFD_CLOEXEC` permission, not a missing
    // syscall: which syscalls a forked child issues is libc-dependent, so
    // removing one tests nothing on a libc that does not use it (dropping
    // `set_robust_list` was a no-op on musl). Every libc needs to clone a
    // descriptor here, so denying that is a gap everywhere.
    let full: Vec<libc::c_long> = BASELINE.iter().chain(FORK_SERVER_EXTRA).copied().collect();
    // `fork_server: false` here: the gap under test is the missing
    // `F_DUPFD_CLOEXEC`, and the `clone` argument-filter is orthogonal to it.
    if install_with(full, false, false, false).is_err() {
        eprintln!("could not install the crippled filter");
        std::process::exit(2);
    }
    // Must not return: the canary is expected to abort the process.
    verify_fork_server_filter();
    eprintln!("canary did NOT detect the missing syscall");
    std::process::exit(3);
}

/// Distinct child exit codes, so a canary failure names the operation rather
/// than just reporting "the child died".
#[cfg(feature = "multi-process")]
const EXIT_CANARY_DUP: libc::c_int = 91;
#[cfg(feature = "multi-process")]
const EXIT_CANARY_SECCOMP: libc::c_int = 92;

/// Fail closed, matching the rest of this module: a sandbox that cannot be
/// shown to work is treated exactly like one that failed to install.
#[cfg(feature = "multi-process")]
fn fail_canary(detail: &str) -> ! {
    eprintln!("[fork-server] FATAL: sandbox self-check failed: {detail}");
    eprintln!(
        "[fork-server] renderers would crash on spawn; refusing to continue. \
               Use --single-process on this host."
    );
    std::process::exit(1);
}

/// Cap the fork server: the baseline, plus forking, reaping, and the one
/// `fcntl` command a freshly-forked renderer needs before its own lockdown.
#[cfg(feature = "multi-process")]
pub fn lock_down_fork_server() {
    deny_debugger_attach();
    // clone3 -> ENOSYS pre-filter must stack first, so glibc's `fork()` falls
    // back to register-based `clone`, which the main filter can constrain.
    // `clone3` cannot be argument-filtered (its flags live in a struct seccomp
    // can't read), so without this pre-filter it stays allowed and
    // unconstrained, reopening the CLONE_NEWUSER/CLONE_VM/namespace vectors
    // the `clone` rule blocks - hence fail-closed on install failure. A libc
    // that doesn't honour the fallback is caught by `verify_fork_server_filter`.
    if let Err(e) = install_clone3_enosys() {
        eprintln!(
            "[fork-server] FATAL: could not install clone3->ENOSYS pre-filter ({e}); \
             refusing to run with clone3 unconstrained"
        );
        std::process::exit(1);
    }
    let allowed: Vec<libc::c_long> = BASELINE.iter().chain(FORK_SERVER_EXTRA).copied().collect();
    enforce("fork-server", install_fork_server(allowed));
}

/// Cap the fork server for a font system that answered
/// `Confinement::FontPathsReadable`: [`lock_down_fork_server`] plus the
/// file-reading syscalls, with Landlock granting exactly `fs_allow`.
#[cfg(feature = "multi-process")]
pub fn lock_down_fork_server_with_font_access(fs_allow: &[(&std::path::Path, bool)]) {
    deny_debugger_attach();

    if !fs_allow.is_empty() {
        match landlock::restrict(fs_allow) {
            Ok(true) => eprintln!("[fork-server+fonts] landlock active (filesystem scoped to font paths)"),
            Ok(false) => {
                eprintln!("[fork-server+fonts] landlock unavailable on this kernel; filesystem NOT path-scoped")
            }
            Err(e) => {
                eprintln!("[fork-server+fonts] landlock could not be applied ({e}); filesystem NOT path-scoped")
            }
        }
    }

    // Same fail-closed clone3 story as `lock_down_fork_server` - see there.
    if let Err(e) = install_clone3_enosys() {
        eprintln!(
            "[fork-server+fonts] FATAL: could not install clone3->ENOSYS pre-filter ({e}); \
             refusing to run with clone3 unconstrained"
        );
        std::process::exit(1);
    }
    let allowed: Vec<libc::c_long> = BASELINE
        .iter()
        .chain(FORK_SERVER_EXTRA)
        .chain(FS_EXTRA)
        .chain(FONT_READ_EXTRA)
        .copied()
        .collect();
    enforce("fork-server+fonts", install_fork_server(allowed));
}

/// Cap a renderer that was forked from the fork server, to the tier its
/// font system answered: the plain renderer baseline, or - with `font_access`
/// - the baseline plus the file-reading syscalls.
#[cfg(feature = "multi-process")]
pub fn lock_down_forked_renderer(font_access: bool) {
    let mut allowed = BASELINE.to_vec();
    if font_access {
        allowed.extend_from_slice(FS_EXTRA);
        allowed.extend_from_slice(FONT_READ_EXTRA);
    }
    let role = if font_access {
        "renderer+fonts (forked)"
    } else {
        "renderer (forked)"
    };
    enforce(role, install(allowed));
}

/// Which side of a [`fork_process`] call this process is.
#[cfg(feature = "multi-process")]
pub enum Forked {
    /// The original process; `pid` is the child to eventually [`reap_child`].
    Parent { pid: i32 },
    /// The new process. It must do its work and leave via [`exit_now`] - never
    /// return into the caller's stack, which belongs to the parent's logic.
    Child,
}

/// Fork the calling process.
#[cfg(feature = "multi-process")]
pub fn fork_process() -> std::io::Result<Forked> {
    // SAFETY: fork itself is always safe to *call*; the single-threaded-caller
    // requirement above is what makes the child side usable, and the one
    // caller (the fork server) upholds it.
    let pid = unsafe { libc::fork() };
    match pid {
        p if p < 0 => Err(std::io::Error::last_os_error()),
        0 => Ok(Forked::Child),
        p => Ok(Forked::Parent { pid: p }),
    }
}

/// Wait for a forked child and return its raw wait status.
#[cfg(feature = "multi-process")]
pub fn reap_child(pid: i32) -> std::io::Result<i32> {
    let mut status: libc::c_int = 0;
    // SAFETY: `status` is a valid out-parameter; `pid` is our own child.
    if unsafe { libc::waitpid(pid, &mut status, 0) } != pid {
        return Err(std::io::Error::last_os_error());
    }
    Ok(status)
}

/// Exit immediately, without running destructors or `atexit` handlers.
#[cfg(feature = "multi-process")]
pub fn exit_now(code: i32) -> ! {
    // SAFETY: `_exit` is async-signal-safe and takes no pointers.
    unsafe { libc::_exit(code) }
}

/// The argv area of this process (`arg_start`..`arg_end` from
/// `/proc/self/stat`), captured before any lockdown so [`set_process_title`]
/// can rewrite it without a syscall. Forked children inherit the capture.
#[cfg(feature = "multi-process")]
static PROCESS_TITLE_REGION: std::sync::OnceLock<(usize, usize)> = std::sync::OnceLock::new();

/// Record where this process's argv lives. Reads `/proc`, so it must run
/// before any filter or Landlock ruleset takes file opens away; after
/// lockdown it quietly captures nothing and [`set_process_title`] falls back
/// to renaming the comm only.
#[cfg(feature = "multi-process")]
pub fn capture_process_title_region() {
    if PROCESS_TITLE_REGION.get().is_some() {
        return;
    }
    let Ok(stat) = std::fs::read_to_string("/proc/self/stat") else {
        return;
    };
    // Fields are positional after the parenthesized comm (which may contain
    // spaces): the slice after ')' starts at field 3, so arg_start/arg_end
    // (fields 48/49, proc(5)) sit at indices 45/46.
    let Some(close) = stat.rfind(')') else { return };
    let fields: Vec<&str> = stat[close + 1..].split_whitespace().collect();
    let (Some(start), Some(end)) = (
        fields.get(45).and_then(|s| s.parse::<usize>().ok()),
        fields.get(46).and_then(|s| s.parse::<usize>().ok()),
    ) else {
        return;
    };
    if start == 0 || end <= start {
        return;
    }
    let _ = PROCESS_TITLE_REGION.set((start, end));
}

/// Rename this process in `ps`/`pstree`: `comm` (at most 15 bytes, what
/// `pstree` prints by default) via `prctl(PR_SET_NAME)`, and the cmdline by
/// overwriting the captured argv area in place - plain memory writes, so it
/// works under every filter here (`PR_SET_NAME` is on each allowlist too).
/// Built for the fork-without-exec children, which otherwise show their
/// parent's cmdline.
#[cfg(feature = "multi-process")]
pub fn set_process_title(comm: &str, cmdline: &str) {
    let mut name = [0u8; 16];
    let n = comm.len().min(15);
    name[..n].copy_from_slice(&comm.as_bytes()[..n]);
    // SAFETY: PR_SET_NAME reads a NUL-terminated buffer of at most 16 bytes.
    unsafe { libc::prctl(libc::PR_SET_NAME, name.as_ptr()) };

    let Some(&(start, end)) = PROCESS_TITLE_REGION.get() else {
        return;
    };
    let len = end - start;
    let bytes = cmdline.as_bytes();
    let n = bytes.len().min(len.saturating_sub(1));
    // SAFETY: arg_start..arg_end is this process's own writable argv memory
    // (initial stack); writing within it is the standard setproctitle move.
    // The remainder is zeroed so readers see exactly the new title.
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), start as *mut u8, n);
        std::ptr::write_bytes((start + n) as *mut u8, 0, len - n);
    }
}

/// The write end of the pipe the PID-namespace anchor blocks on. Dropping it
/// (or exiting) is what releases the anchor: the read gets EOF and PID 1
/// exits, tearing the namespace down with the fork server.
#[cfg(feature = "multi-process")]
pub struct PidNamespaceAnchor {
    write_fd: libc::c_int,
}

#[cfg(feature = "multi-process")]
impl Drop for PidNamespaceAnchor {
    fn drop(&mut self) {
        // SAFETY: closing a descriptor this struct exclusively owns.
        unsafe { libc::close(self.write_fd) };
    }
}

/// Park a child as PID 1 of the fork server's PID namespace, for as long as
/// the returned anchor lives.
#[cfg(feature = "multi-process")]
pub fn hold_pid_namespace_anchor() -> std::io::Result<PidNamespaceAnchor> {
    let mut fds: [libc::c_int; 2] = [0; 2];
    // SAFETY: `fds` is a valid two-slot out-array.
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    let (read_fd, write_fd) = (fds[0], fds[1]);

    match fork_process()? {
        Forked::Child => {
            // SAFETY: closing the inherited copy of the parent's end.
            unsafe { libc::close(write_fd) };
            set_process_title("pidns-anchor", "gosub: pid-namespace anchor");
            // Confine quietly (no lockdown banner - this is plumbing, not a
            // component); an install failure leaves an idle read loop, which
            // is not worth killing the namespace over.
            let _ = install(BASELINE.to_vec());
            loop {
                let mut byte = 0u8;
                // SAFETY: reading one byte into a valid buffer from an fd we own.
                let n = unsafe { libc::read(read_fd, std::ptr::addr_of_mut!(byte).cast(), 1) };
                let interrupted = n < 0 && std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted;
                if !interrupted {
                    break;
                }
            }
            exit_now(0);
        }
        Forked::Parent { pid: _ } => {
            // The anchor is reaped by namespace teardown, not by us: it outlives
            // every renderer and exits only as the fork server does.
            // SAFETY: closing the inherited copy of the child's end.
            unsafe { libc::close(read_fd) };
            Ok(PidNamespaceAnchor { write_fd })
        }
    }
}

/// Cap a renderer: pixels only - the baseline, no network, files, or exec.
#[cfg(feature = "multi-process")]
pub fn lock_down_renderer() {
    deny_debugger_attach();
    enforce("renderer", install(BASELINE.to_vec()));
}

/// The image decoder's confinement: the renderer's pixels-only baseline -
/// it needs exactly as little - under its own banner name.
#[cfg(feature = "multi-process")]
pub fn lock_down_decoder() {
    deny_debugger_attach();
    enforce("decoder", install(BASELINE.to_vec()));
}

/// What a fontconfig-backed font stack does at match time, beyond opening the
/// font file: probes cache freshness (`access`, or `faccessat` on
/// architectures without it), lists font directories, follows their symlinks.
/// Empirically derived against Pango and Skia (a missing entry surfaces as
/// SIGSYS naming the syscall); expected to grow per libc like
/// [`NET_RUNTIME_EXTRA`].
#[cfg(feature = "multi-process")]
const FONT_READ_EXTRA: &[libc::c_long] = &[
    #[cfg(any(target_arch = "x86_64", target_arch = "x86"))]
    libc::SYS_access,
    libc::SYS_faccessat,
    libc::SYS_faccessat2,
    libc::SYS_getdents64,
    libc::SYS_readlinkat,
    #[cfg(any(target_arch = "x86_64", target_arch = "x86"))]
    libc::SYS_readlink,
    // Skia's fontconfig wrapper resolves relative font paths against the
    // working directory. Reads a string the process already owns.
    libc::SYS_getcwd,
    // Skia probes the filesystem a font lives on before mapping it (fstatfs on
    // the open fd, statfs on the path). Filesystem metadata only.
    libc::SYS_fstatfs,
    libc::SYS_statfs,
    // Readahead hint on the font file before mapping it. Advisory only.
    libc::SYS_fadvise64,
];

/// Cap a renderer whose font system must read font files: the renderer
/// baseline plus the file-reading syscalls, with Landlock deciding which
/// paths they may reach (pass [`font_filesystem_paths`], read-only).
#[cfg(feature = "multi-process")]
pub fn lock_down_renderer_with_font_access(fs_allow: &[(&std::path::Path, bool)]) {
    deny_debugger_attach();

    // Landlock before seccomp, as everywhere: its own syscalls and the O_PATH
    // anchor opens must run unfiltered.
    if !fs_allow.is_empty() {
        match landlock::restrict(fs_allow) {
            Ok(true) => eprintln!("[renderer+fonts] landlock active (filesystem scoped to font paths)"),
            Ok(false) => {
                eprintln!("[renderer+fonts] landlock unavailable on this kernel; filesystem NOT path-scoped")
            }
            Err(e) => {
                eprintln!("[renderer+fonts] landlock could not be applied ({e}); filesystem NOT path-scoped")
            }
        }
    }

    let mut allowed = BASELINE.to_vec();
    allowed.extend_from_slice(FS_EXTRA);
    allowed.extend_from_slice(FONT_READ_EXTRA);
    enforce("renderer+fonts", install(allowed));
}

/// The read-only paths a fontconfig-backed font stack needs on a typical Linux
/// system: the font directories, the fontconfig configuration, and its caches.
/// Only paths that exist are returned, like [`net_filesystem_paths`], and the
/// list is expected to grow per distribution the same way.
#[cfg(feature = "multi-process")]
pub fn font_filesystem_paths() -> Vec<std::path::PathBuf> {
    let mut candidates: Vec<std::path::PathBuf> = [
        // Font files, as the major distributions arrange them.
        "/usr/share/fonts",
        "/usr/local/share/fonts",
        // fontconfig's configuration tree and system-wide cache.
        "/etc/fonts",
        "/usr/share/fontconfig",
        "/var/cache/fontconfig",
    ]
    .iter()
    .map(std::path::PathBuf::from)
    .collect();

    // Per-user fonts and the per-user fontconfig cache. `HOME` rather than a
    // full XDG resolution: this list only needs to cover what fontconfig will
    // actually probe, and missing entries surface as a Landlock denial naming
    // the path.
    if let Some(home) = std::env::var_os("HOME").map(std::path::PathBuf::from) {
        candidates.push(home.join(".fonts"));
        candidates.push(home.join(".local/share/fonts"));
        candidates.push(home.join(".cache/fontconfig"));
    }
    if let Some(xdg_cache) = std::env::var_os("XDG_CACHE_HOME").map(std::path::PathBuf::from) {
        candidates.push(xdg_cache.join("fontconfig"));
    }

    candidates.retain(|p| p.exists());
    candidates.dedup();
    candidates
}

/// Cap the vault (cookie store): the same bare baseline as a renderer - no
/// network, `openat`, `ioctl`, or exec. It only moves bytes on its inherited
/// IPC fd and touches its own memory.
#[cfg(feature = "multi-process")]
pub fn lock_down_vault() {
    deny_debugger_attach();
    enforce("vault", install(BASELINE.to_vec()));
}

/// Cap the net component: the baseline plus the socket family.
#[cfg(feature = "multi-process")]
pub fn lock_down_net(fs_allow: &[(&std::path::Path, bool)]) {
    deny_debugger_attach();

    // TLS needs the system root certificates and DNS the resolver config;
    // denying `openat` outright killed the process on its first HTTPS request.
    // So `openat` is allowed and Landlock decides which paths it may reach:
    // seccomp bounds the operation, Landlock bounds the target. Landlock's
    // absence is reported, since without it the filesystem is unscoped.
    if !fs_allow.is_empty() {
        match landlock::restrict(fs_allow) {
            Ok(true) => eprintln!("[net] landlock active (filesystem scoped to resolver and CA paths)"),
            Ok(false) => eprintln!("[net] landlock unavailable on this kernel; filesystem NOT path-scoped"),
            Err(e) => eprintln!("[net] landlock could not be applied ({e}); filesystem NOT path-scoped"),
        }
    }

    let allowed: Vec<libc::c_long> = BASELINE
        .iter()
        .chain(NET_EXTRA)
        .chain(NET_RUNTIME_EXTRA)
        .chain(FS_EXTRA)
        .copied()
        .collect();
    // Socket-mode fcntls permitted: the runtime toggles O_NONBLOCK per socket.
    enforce("net", install_with(allowed, false, false, true));
}

/// The read-only paths a network stack needs on a typical Linux system:
/// resolver configuration and the system trust store. Only existing paths are
/// returned (Landlock rules name real objects; distributions differ). A
/// missing entry surfaces as SIGSYS on `openat` or a failing TLS handshake;
/// expected to grow per distribution.
#[cfg(feature = "multi-process")]
pub fn net_filesystem_paths() -> Vec<std::path::PathBuf> {
    const CANDIDATES: &[&str] = &[
        // Resolver configuration.
        "/etc/resolv.conf",
        "/etc/hosts",
        "/etc/nsswitch.conf",
        "/etc/gai.conf",
        "/etc/host.conf",
        "/etc/services",
        // System trust stores, as the major distributions arrange them.
        "/etc/ssl",
        "/etc/pki",
        "/etc/ca-certificates",
        "/usr/share/ca-certificates",
        "/usr/local/share/ca-certificates",
        "/etc/ssl/certs/ca-certificates.crt",
    ];

    CANDIDATES
        .iter()
        .map(std::path::PathBuf::from)
        .filter(|p| p.exists())
        .collect()
}

/// The syscalls a filesystem-capable service needs beyond the baseline to open
/// a file. Renderers deny these outright; font/storage services need them.
#[cfg(feature = "multi-process")]
const FS_EXTRA: &[libc::c_long] = &[
    libc::SYS_openat,
    #[cfg(any(target_arch = "x86_64", target_arch = "x86"))]
    libc::SYS_open,
];

/// Landlock: path-based filesystem access control (which seccomp cannot do).
#[cfg(feature = "multi-process")]
mod landlock {
    use std::os::fd::RawFd;
    use std::os::unix::ffi::OsStrExt;
    use std::path::Path;

    // Access-right bits (ABI v1 unless noted). From the Landlock uapi.
    const EXECUTE: u64 = 1 << 0;
    const WRITE_FILE: u64 = 1 << 1;
    const READ_FILE: u64 = 1 << 2;
    const READ_DIR: u64 = 1 << 3;
    const REMOVE_DIR: u64 = 1 << 4;
    const REMOVE_FILE: u64 = 1 << 5;
    const MAKE_CHAR: u64 = 1 << 6;
    const MAKE_DIR: u64 = 1 << 7;
    const MAKE_REG: u64 = 1 << 8;
    const MAKE_SOCK: u64 = 1 << 9;
    const MAKE_FIFO: u64 = 1 << 10;
    const MAKE_BLOCK: u64 = 1 << 11;
    const MAKE_SYM: u64 = 1 << 12;
    const REFER: u64 = 1 << 13; // ABI v2
    const TRUNCATE: u64 = 1 << 14; // ABI v3

    const CREATE_RULESET_VERSION: u32 = 1 << 0;
    const RULE_PATH_BENEATH: libc::c_int = 1;

    #[repr(C)]
    struct RulesetAttr {
        handled_access_fs: u64,
    }

    #[repr(C)]
    struct PathBeneathAttr {
        allowed_access: u64,
        parent_fd: RawFd,
    }

    /// The supported ABI version, or `-1`/`0` when Landlock is unavailable.
    fn abi() -> i32 {
        // SAFETY: create_ruleset(NULL, 0, VERSION) is the documented probe; it
        // returns the ABI version and creates nothing.
        unsafe {
            libc::syscall(
                libc::SYS_landlock_create_ruleset,
                std::ptr::null::<RulesetAttr>(),
                0usize,
                CREATE_RULESET_VERSION,
            ) as i32
        }
    }

    /// Whether Landlock is usable on this kernel.
    pub fn available() -> bool {
        abi() >= 1
    }

    /// Every fs right this ABI knows - the set the ruleset *handles* (anything
    /// handled but not granted by a rule is denied). Masked to the ABI so an
    /// unsupported bit does not make `create_ruleset` fail.
    fn handled(abi: i32) -> u64 {
        let mut h = EXECUTE
            | WRITE_FILE
            | READ_FILE
            | READ_DIR
            | REMOVE_DIR
            | REMOVE_FILE
            | MAKE_CHAR
            | MAKE_DIR
            | MAKE_REG
            | MAKE_SOCK
            | MAKE_FIFO
            | MAKE_BLOCK
            | MAKE_SYM;
        if abi >= 2 {
            h |= REFER;
        }
        if abi >= 3 {
            h |= TRUNCATE;
        }
        h
    }

    /// Rights to grant one *service* path. Directory-only rights (`READ_DIR`,
    /// `MAKE_REG`, `REMOVE_FILE`) must not be set on a *file* path or `add_rule`
    /// rejects the ruleset with `EINVAL` - so the grant depends on `is_dir`.
    /// `TRUNCATE` (ABI v3) is included unconditionally; [`apply`] masks it off on
    /// older kernels.
    fn grant(is_dir: bool, writable: bool) -> u64 {
        let mut a = READ_FILE;
        if is_dir {
            a |= READ_DIR;
        }
        if writable {
            a |= WRITE_FILE | TRUNCATE;
            if is_dir {
                // Create and remove entries under the directory.
                a |= MAKE_REG | REMOVE_FILE;
            }
        }
        a
    }

    /// Directory-only rights - invalid on a *file* path, so [`apply`] strips them
    /// there rather than let one file rule `EINVAL` the whole ruleset.
    const DIR_ONLY: u64 = READ_DIR
        | MAKE_REG
        | MAKE_DIR
        | REMOVE_FILE
        | REMOVE_DIR
        | MAKE_CHAR
        | MAKE_SOCK
        | MAKE_FIFO
        | MAKE_BLOCK
        | MAKE_SYM;

    /// Create a ruleset handling all fs access, add each `(path, rights)` rule
    /// (rights masked to this ABI, and to what the path - file vs directory -
    /// can carry), then enforce it on the calling thread and everything it later
    /// spawns. `Ok(true)` = applied, `Ok(false)` = Landlock unavailable (caller
    /// degrades), `Err` = a real failure.
    fn apply(rules: &[(&Path, u64)]) -> std::io::Result<bool> {
        let abi = abi();
        if abi < 1 {
            return Ok(false);
        }
        let attr = RulesetAttr {
            handled_access_fs: handled(abi),
        };
        // SAFETY: valid attr pointer with its size; flags 0.
        let rs = unsafe {
            libc::syscall(
                libc::SYS_landlock_create_ruleset,
                &attr as *const RulesetAttr,
                std::mem::size_of::<RulesetAttr>(),
                0u32,
            )
        };
        if rs < 0 {
            return Err(std::io::Error::last_os_error());
        }
        let rs = rs as RawFd;

        for (path, rights) in rules {
            let cpath = std::ffi::CString::new(path.as_os_str().as_bytes())
                .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "NUL in path"))?;
            // SAFETY: NUL-terminated path; O_PATH just anchors the rule.
            let pfd = unsafe { libc::open(cpath.as_ptr(), libc::O_PATH | libc::O_CLOEXEC) };
            if pfd < 0 {
                let e = std::io::Error::last_os_error();
                unsafe { libc::close(rs) };
                return Err(e);
            }
            // Decide dir-ness from the *opened inode* (`fstat` on the `O_PATH`
            // fd), not a second by-name `is_dir()` lookup: a path swapped
            // (file↔dir↔symlink) between the `open` above and the check would
            // otherwise mask `DIR_ONLY` against a different inode than the rule is
            // anchored to. `fstat` on the O_PATH fd is TOCTOU-free. Falls back to
            // "not a dir" (strips `DIR_ONLY`) on any stat error, as before.
            let mut allowed = *rights & handled(abi);
            let mut st: libc::stat = unsafe { std::mem::zeroed() };
            let is_dir = unsafe { libc::fstat(pfd, &mut st) } == 0 && (st.st_mode & libc::S_IFMT) == libc::S_IFDIR;
            if !is_dir {
                allowed &= !DIR_ONLY;
            }
            let rule = PathBeneathAttr {
                allowed_access: allowed,
                parent_fd: pfd,
            };
            // SAFETY: valid ruleset fd, rule pointer, and rule type.
            let rc = unsafe {
                libc::syscall(
                    libc::SYS_landlock_add_rule,
                    rs,
                    RULE_PATH_BENEATH,
                    &rule as *const PathBeneathAttr,
                    0u32,
                )
            };
            unsafe { libc::close(pfd) };
            if rc < 0 {
                let e = std::io::Error::last_os_error();
                unsafe { libc::close(rs) };
                return Err(e);
            }
        }

        // restrict_self requires NO_NEW_PRIVS (the seccomp install would set it
        // too, but that runs later - and the broker never installs seccomp).
        // SAFETY: a one-way prctl switch.
        if unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) } < 0 {
            let e = std::io::Error::last_os_error();
            unsafe { libc::close(rs) };
            return Err(e);
        }
        // SAFETY: valid ruleset fd; flags 0.
        let rc = unsafe { libc::syscall(libc::SYS_landlock_restrict_self, rs, 0u32) };
        let e = std::io::Error::last_os_error();
        unsafe { libc::close(rs) };
        if rc < 0 {
            return Err(e);
        }
        Ok(true)
    }

    /// Restrict this thread's filesystem access to exactly `rules`
    /// `(path, writable)` - the *service* confinement (read, plus write on a
    /// `writable` path).
    pub fn restrict(rules: &[(&Path, bool)]) -> std::io::Result<bool> {
        let mapped: Vec<(&Path, u64)> = rules.iter().map(|(p, w)| (*p, grant(p.is_dir(), *w))).collect();
        apply(&mapped)
    }

    /// The *broker* confinement - a loose sandbox for the engine process.
    pub fn restrict_broker(temp: &Path, cgroup: Option<&Path>) -> std::io::Result<bool> {
        // Read + traverse + execute everything, so the loader can `execve` the
        // child binary and mmap its shared libraries PROT_EXEC wherever they are.
        let root = READ_FILE | READ_DIR | EXECUTE;
        // Full write beneath the temp dir: create/remove the storage dir and the
        // font file, and write/truncate them.
        let temp_rw = READ_FILE | READ_DIR | WRITE_FILE | TRUNCATE | MAKE_REG | MAKE_DIR | REMOVE_FILE | REMOVE_DIR;
        let mut rules: Vec<(&Path, u64)> = vec![(Path::new("/"), root), (temp, temp_rw)];
        // When cgroup memory bounding is active, the broker must keep writing
        // under its `workers` subtree to place each child: make per-child cgroup
        // dirs and write their `memory.*` / `cgroup.procs` interface files. No
        // `MAKE_REG` - the kernel materialises those files when the dir is made.
        if let Some(cg) = cgroup {
            let cg_rw = READ_FILE | READ_DIR | WRITE_FILE | MAKE_DIR | REMOVE_DIR;
            rules.push((cg, cg_rw));
        }
        apply(&rules)
    }
}

/// Whether Landlock is usable on this host (for probes and diagnostics).
#[cfg(feature = "multi-process")]
pub fn landlock_available() -> bool {
    landlock::available()
}

/// cgroup v2 per-child memory bounding - the physical-memory limit
/// `RLIMIT_AS`/`RLIMIT_DATA` cannot give.
#[cfg(feature = "multi-process")]
mod cgroup {
    use std::io::Write;
    use std::path::{Path, PathBuf};
    use std::sync::OnceLock;

    /// The `…/workers` cgroup per-child limits live under, set once by
    /// [`prepare`]. `Some(None)` ⇒ tried and unavailable; unset ⇒ never prepared.
    /// Either way [`place_child`] degrades to a no-op.
    static WORKERS: OnceLock<Option<PathBuf>> = OnceLock::new();

    /// Graceful-reclaim threshold (`memory.high`) and the hard ceiling
    /// (`memory.max`, 25% headroom, where the scoped OOM kill fires). Values
    /// are illustrative for the PoC.
    const HIGH_BYTES: u64 = 1024 * 1024 * 1024;
    const MAX_BYTES: u64 = HIGH_BYTES + HIGH_BYTES / 4;

    /// This process's cgroup-v2 directory, from the sole `0::<path>` line of
    /// `/proc/self/cgroup`. `None` if the host is cgroup v1 / hybrid.
    fn my_dir() -> Option<PathBuf> {
        let content = std::fs::read_to_string("/proc/self/cgroup").ok()?;
        let rel = content.lines().find_map(|l| l.strip_prefix("0::"))?;
        Some(Path::new("/sys/fs/cgroup").join(rel.trim().trim_start_matches('/')))
    }

    fn write_file(path: &Path, val: &str) -> std::io::Result<()> {
        std::fs::OpenOptions::new()
            .write(true)
            .open(path)?
            .write_all(val.as_bytes())
    }

    /// Best-effort leader-pattern setup, run on the broker's main thread before
    /// its Landlock/seccomp go on. Returns the `workers` dir if the memory
    /// controller could be delegated to our own subtree, else `None`. Also
    /// records the result for [`place_child`].
    pub fn prepare() -> Option<PathBuf> {
        let workers = try_prepare();
        let _ = WORKERS.set(workers.clone());
        workers
    }

    fn try_prepare() -> Option<PathBuf> {
        let base = my_dir()?;
        // The memory controller must have been delegated down to our leaf.
        let controllers = std::fs::read_to_string(base.join("cgroup.controllers")).ok()?;
        if !controllers.split_whitespace().any(|c| c == "memory") {
            return None;
        }
        // Unique per broker pid, so parallel binaries in a shared scope don't
        // collide on the same directory names.
        let pid = std::process::id();
        let leader = base.join(format!("gosub.{pid}.leader"));
        let workers = base.join(format!("gosub.{pid}.workers"));
        let _ = std::fs::create_dir(&leader);
        let _ = std::fs::create_dir(&workers);
        // Move the whole thread group into the leader leaf, emptying our own
        // cgroup so it may delegate controllers.
        if write_file(&leader.join("cgroup.procs"), &pid.to_string()).is_err() {
            undo(&base, &leader, &workers, pid);
            return None;
        }
        // Delegate memory down to `workers`. The first write fails `EBUSY` if our
        // leaf still has other processes (a shared scope) - the fallback trigger.
        if write_file(&base.join("cgroup.subtree_control"), "+memory").is_err()
            || write_file(&workers.join("cgroup.subtree_control"), "+memory").is_err()
        {
            undo(&base, &leader, &workers, pid);
            return None;
        }
        Some(workers)
    }

    /// Move back to our original cgroup and remove the (empty) leaves, so a
    /// fallback leaves no trace in a shared scope.
    fn undo(base: &Path, leader: &Path, workers: &Path, pid: u32) {
        let _ = write_file(&base.join("cgroup.procs"), &pid.to_string());
        let _ = std::fs::remove_dir(workers);
        let _ = std::fs::remove_dir(leader);
    }

    /// Tear the subtree down at broker shutdown, symmetric to [`prepare`]. Called
    /// once the broker has reaped every child, so the per-child cgroups are empty
    /// and removable.
    pub fn cleanup() {
        let Some(Some(workers)) = WORKERS.get() else { return };
        // Per-child leaves (and any self-test `probe` leaf) - Landlock-permitted.
        let mut removed = 0usize;
        if let Ok(entries) = std::fs::read_dir(workers) {
            for entry in entries.flatten() {
                if std::fs::remove_dir(entry.path()).is_ok() {
                    removed += 1;
                }
            }
        }
        if std::env::var_os("GOSUB_DEBUG_CGROUP").is_some() {
            eprintln!("[broker] cgroup: cleanup removed {removed} per-child leaf cgroup(s)");
        }
        // Best-effort full teardown; the parent-cgroup steps no-op under Landlock.
        if let Some(base) = workers.parent() {
            let pid = std::process::id();
            let leader = base.join(format!("gosub.{pid}.leader"));
            let _ = write_file(&base.join("cgroup.procs"), &pid.to_string());
            let _ = std::fs::remove_dir(workers);
            let _ = std::fs::remove_dir(&leader);
        }
    }

    /// Place a freshly-spawned child into its own memory-limited cgroup. A no-op
    /// where the subtree was never set up, and only logged (never fatal) on error
    /// - the child still runs, just rlimit-bounded, exactly as before cgroups.
    pub fn place_child(pid: u32) {
        let Some(Some(workers)) = WORKERS.get() else { return };
        let dir = workers.join(format!("c-{pid}"));
        if let Err(e) = (|| -> std::io::Result<()> {
            let _ = std::fs::create_dir(&dir);
            write_file(&dir.join("memory.max"), &MAX_BYTES.to_string())?;
            // Graceful reclaim before the hard cap; best-effort (old kernels).
            let _ = write_file(&dir.join("memory.high"), &HIGH_BYTES.to_string());
            // Writing the pid moves the child out of the inherited leader cgroup
            // into its own bounded one.
            write_file(&dir.join("cgroup.procs"), &pid.to_string())
        })() {
            eprintln!("[broker] cgroup: could not confine child {pid} ({e}); it runs rlimit-bounded only");
        } else if std::env::var_os("GOSUB_DEBUG_CGROUP").is_some() {
            eprintln!(
                "[broker] cgroup: child {pid} confined to {} (memory.max={MAX_BYTES})",
                dir.display()
            );
        }
    }

    /// Test hook: set up the subtree (best-effort) and place *this* process in a
    /// child cgroup limited to `limit`, returning the value read back from
    /// `memory.max` - or `None` if cgroup v2 memory delegation is unavailable, so
    /// the probe skips cleanly like the Landlock one.
    pub fn confine_self(limit: u64) -> Option<std::io::Result<u64>> {
        let workers = prepare()?;
        Some((|| {
            let dir = workers.join("probe");
            let _ = std::fs::create_dir(&dir);
            write_file(&dir.join("memory.max"), &limit.to_string())?;
            write_file(&dir.join("cgroup.procs"), &std::process::id().to_string())?;
            std::fs::read_to_string(dir.join("memory.max"))?
                .trim()
                .parse::<u64>()
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
        })())
    }
}

/// Place a just-spawned child into its own cgroup memory limit (best-effort; see
/// [`cgroup`]). The Linux half of the parent-side [`crate::confine_spawned_child`]
/// seam - the analogue of the Windows job-object memory cap.
#[cfg(feature = "multi-process")]
pub fn confine_spawned_child(pid: u32) -> std::io::Result<()> {
    cgroup::place_child(pid);
    Ok(())
}

/// Tear down the broker's cgroup subtree at shutdown (best-effort), symmetric to
/// the per-child placement in [`confine_spawned_child`]. Call once every child
/// has been reaped. See [`cgroup::cleanup`].
#[cfg(feature = "multi-process")]
pub fn cleanup_spawned_cgroups() {
    cgroup::cleanup();
}

/// Test hook for the `cgroup-memory-limit` probe: bound this process's memory via
/// cgroup v2 and read the ceiling back, or `None` where delegation is
/// unavailable. See [`cgroup::confine_self`].
#[cfg(feature = "multi-process")]
pub fn cgroup_confine_self(limit: u64) -> Option<std::io::Result<u64>> {
    cgroup::confine_self(limit)
}

/// Syscalls denied to the broker (engine). The broker execs helpers, spawns
/// threads, and opens files and sockets, so an allowlist does not fit
/// (Chromium's browser process is likewise not seccomp-allowlisted); instead
/// it is denied the post-compromise escalation primitives it never needs -
/// see the per-group comments below.
#[cfg(feature = "multi-process")]
const BROKER_DENY: &[libc::c_long] = &[
    // Attach to / read / write another process - injection and secret theft.
    libc::SYS_ptrace,
    libc::SYS_process_vm_readv,
    libc::SYS_process_vm_writev,
    // Load kernel code - the shortest path from a broker compromise to ring 0.
    libc::SYS_kexec_load,
    libc::SYS_kexec_file_load,
    libc::SYS_init_module,
    libc::SYS_finit_module,
    libc::SYS_delete_module,
    libc::SYS_bpf,
    // Classic LPE / exploit-primitive surfaces.
    libc::SYS_perf_event_open,
    libc::SYS_userfaultfd,
    libc::SYS_add_key,
    libc::SYS_request_key,
    libc::SYS_keyctl,
    libc::SYS_kcmp,
    // Namespace / mount escapes (the broker uses `unshare`, never these).
    libc::SYS_setns,
    libc::SYS_mount,
    libc::SYS_umount2,
    libc::SYS_pivot_root,
    libc::SYS_swapon,
    libc::SYS_swapoff,
    libc::SYS_reboot,
    // The newer *fd-based* mount API (Linux 5.1+) - the same capability as
    // `mount`/`pivot_root` reached a different way, so denying only the classic
    // calls would leave the escape open via `fsopen`+`fsconfig`+`fsmount`+
    // `move_mount` (or `open_tree` for a detached mount, `mount_setattr` to
    // remount). Denied together so the mount surface is closed as a whole.
    libc::SYS_fsopen,
    libc::SYS_fsconfig,
    libc::SYS_fsmount,
    libc::SYS_move_mount,
    libc::SYS_open_tree,
    libc::SYS_fspick,
    libc::SYS_mount_setattr,
    // Open a file straight from a handle, bypassing the path-based access checks
    // (and the broker's Landlock, which sees paths, not handles). No child needs
    // it; the broker never does.
    libc::SYS_open_by_handle_at,
];

/// Confine the broker (engine) process. Two best-effort layers, applied on
/// the main thread before it spawns anything so every thread and child
/// inherits both: Landlock (read/execute anywhere, write only beneath the
/// temp dir - see [`landlock::restrict_broker`]) and a deny-list seccomp
/// filter (allow by default, `Trap` the [`BROKER_DENY`] escalation syscalls).
#[cfg(feature = "multi-process")]
pub fn lock_down_broker() {
    // cgroup memory bounding first (best-effort): it moves the broker into a
    // leader cgroup and writes to `/sys/fs/cgroup`, so it must run *before*
    // Landlock/seccomp go on. The `workers` path it returns, if any, is handed to
    // Landlock so the later per-child placement writes stay allowed.
    let workers = cgroup::prepare();
    match &workers {
        Some(w) => {
            eprintln!(
                "[broker] cgroup memory limits active (children capped under {})",
                w.display()
            )
        }
        None => {
            eprintln!("[broker] cgroup v2 memory delegation unavailable; children fall back to rlimits")
        }
    }

    let temp = std::env::temp_dir();
    match landlock::restrict_broker(&temp, workers.as_deref()) {
        Ok(true) => {
            eprintln!("[broker] landlock active (writes confined to {})", temp.display())
        }
        Ok(false) => {
            eprintln!("[broker] landlock unavailable on this kernel; broker filesystem unconfined")
        }
        Err(e) => {
            eprintln!("[broker] landlock could not be applied ({e}); broker filesystem unconfined")
        }
    }

    // Seccomp after Landlock: the deny-list is default-allow, so it never
    // blocks Landlock's own setup syscalls; same order as the services.
    match install_broker_seccomp() {
        Ok(()) => eprintln!("[broker] seccomp deny-list active (escalation syscalls denied, SIGSYS + report)"),
        Err(e) => eprintln!("[broker] seccomp deny-list could not be applied ({e}); broker syscall surface unconfined"),
    }
}

/// Install the broker's deny-list seccomp filter: allow by default, `Trap`
/// (→ SIGSYS, named by [`install_sigsys_reporter`], then re-raised) on any
/// [`BROKER_DENY`] syscall. The inverse polarity of [`install_with`]'s
/// allowlist - default action `Allow`, matched action `Trap` - so listing a
/// syscall *denies* it and everything unlisted passes.
#[cfg(feature = "multi-process")]
fn install_broker_seccomp() -> Result<(), Box<dyn std::error::Error>> {
    use seccompiler::{apply_filter_all_threads, BpfProgram, SeccompAction, SeccompFilter, SeccompRule};
    use std::collections::BTreeMap;

    #[cfg(target_arch = "x86_64")]
    let arch = seccompiler::TargetArch::x86_64;
    #[cfg(target_arch = "aarch64")]
    let arch = seccompiler::TargetArch::aarch64;

    // Each denied syscall matches unconditionally (an empty rule vec); every
    // other syscall falls through to the default `Allow`.
    let rules: BTreeMap<i64, Vec<SeccompRule>> = BROKER_DENY.iter().map(|&nr| (nr as i64, Vec::new())).collect();

    // Name a denied syscall on stderr before it kills us, exactly as the
    // allowlist path does - "broker tried ptrace (#101), killed" rather than a
    // bare SIGSYS. Its own syscalls are all on the default-allow side here.
    install_sigsys_reporter();
    // …and the crash reporter: on SIGSEGV/ABRT/BUS/ILL/FPE, self-capture a
    // scrubbed, core-less crash report (see `install_crash_reporter`).
    install_crash_reporter();

    let filter = SeccompFilter::new(
        rules,
        SeccompAction::Allow, // default & argument-mismatch: allow (the broker needs breadth)
        SeccompAction::Trap,  // matched (a BROKER_DENY syscall): SIGSYS → report → re-raise
        arch,
    )?;
    let program: BpfProgram = filter.try_into()?;
    // `apply_filter_all` (TSYNC) rather than per-thread: a role's library may
    // have created a thread before its lockdown (measured: Pango's GLib
    // worker), and a filter that missed it would leave one unconfined thread.
    apply_filter_all_threads(&program)?;
    Ok(())
}

/// What a device-backed service (audio, GPU) needs: open a device node and
/// talk to it via `ioctl`. `ioctl` is a large, driver-defined surface seccomp
/// constrains poorly (request codes and pointer args are opaque to the
/// filter); the confinement is the process boundary plus the rest of the
/// baseline.
#[cfg(feature = "multi-process")]
const DEVICE_EXTRA: &[libc::c_long] = &[
    libc::SYS_openat,
    #[cfg(any(target_arch = "x86_64", target_arch = "x86"))]
    libc::SYS_open,
    libc::SYS_ioctl,
];

/// Cap an engine-spawned service - a role that needs a privilege renderers do
/// not, so it lives outside the zygote and carries its own, wider filter. The
/// caps select the superset: `filesystem` adds `openat`, `device` adds
/// `openat` + `ioctl`. Everything else is the same default-deny baseline, so a
/// storage service still cannot open a socket and an audio service still cannot
/// spawn a program.
#[cfg(feature = "multi-process")]
pub fn lock_down_service(name: &str, filesystem: bool, device: bool, fs_allow: &[(&std::path::Path, bool)]) {
    deny_debugger_attach();

    // Landlock first (see the module doc): it runs before the seccomp filter so
    // its own syscalls and the O_PATH opens are unfiltered, and it confines
    // *which* paths the coming `openat` may reach. Best-effort - a kernel
    // without Landlock leaves seccomp + application-level path scoping as the
    // guard rather than refusing to start.
    if !fs_allow.is_empty() {
        match landlock::restrict(fs_allow) {
            Ok(true) => eprintln!("[{name}] landlock active (filesystem scoped to its own paths)"),
            Ok(false) => {
                eprintln!("[{name}] landlock unavailable on this kernel; seccomp + path scoping only")
            }
            Err(e) => eprintln!("[{name}] landlock could not be applied ({e}); seccomp + path scoping only"),
        }
    }

    let mut allowed = BASELINE.to_vec();
    if filesystem {
        allowed.extend_from_slice(FS_EXTRA);
    }
    if device {
        allowed.extend_from_slice(DEVICE_EXTRA);
    }
    enforce(name, install(allowed));
}

#[cfg(feature = "multi-process")]
fn enforce(role: &str, result: Result<(), Box<dyn std::error::Error>>) {
    match result {
        Ok(()) => eprintln!("[{role}] seccomp allowlist active (default-deny, SIGSYS + report)"),
        Err(e) => {
            // Fail closed: never run a component that was meant to be confined
            // as if it were unconfined.
            eprintln!("[{role}] FATAL: could not install seccomp sandbox: {e}");
            std::process::exit(1);
        }
    }
}

/// Build and apply a default-deny allowlist: syscalls in `allowed` pass (subject
/// to any argument filter), every other syscall - and any allowed syscall whose
/// arguments fail its filter - is a fatal `SIGSYS`.
#[cfg(feature = "multi-process")]
fn install(allowed: Vec<libc::c_long>) -> Result<(), Box<dyn std::error::Error>> {
    install_with(allowed, false, false, false)
}

/// The fork server's main filter: as [`install`], but `F_DUPFD_CLOEXEC` is
/// permitted (its forked children clone a descriptor before their own lockdown)
/// and `clone` is argument-filtered to a plain fork (see [`install_with`]).
/// Pair with [`install_clone3_enosys`], installed first.
#[cfg(feature = "multi-process")]
fn install_fork_server(allowed: Vec<libc::c_long>) -> Result<(), Box<dyn std::error::Error>> {
    install_with(allowed, true, true, false)
}

/// As [`install`], but `allow_dup_fd` additionally permits
/// `fcntl(F_DUPFD_CLOEXEC)` and `fork_server` argument-filters `clone` - both
/// needed only by the fork server.
#[cfg(feature = "multi-process")]
fn install_with(
    allowed: Vec<libc::c_long>,
    allow_dup_fd: bool,
    fork_server: bool,
    allow_socket_fcntl: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    use seccompiler::{
        apply_filter_all_threads, BpfProgram, SeccompAction, SeccompCmpArgLen, SeccompCmpOp, SeccompCondition,
        SeccompFilter, SeccompRule,
    };
    use std::collections::BTreeMap;

    #[cfg(target_arch = "x86_64")]
    let arch = seccompiler::TargetArch::x86_64;
    #[cfg(target_arch = "aarch64")]
    let arch = seccompiler::TargetArch::aarch64;

    // Most syscalls match unconditionally: an empty rule vec = any arguments.
    let mut rules: BTreeMap<i64, Vec<SeccompRule>> = allowed.iter().map(|&nr| (nr as i64, Vec::new())).collect();

    // …except mmap/mprotect, which are allowed only when PROT_EXEC is clear.
    // `prot` is argument index 2 of both. `MaskedEq(PROT_EXEC)` against value 0
    // means "(prot & PROT_EXEC) == 0" - so a mapping can be made writable or
    // readable, but never executable (W^X). A request that sets PROT_EXEC
    // matches no rule and hits the KillProcess default.
    for nr in [libc::SYS_mmap, libc::SYS_mprotect] {
        let no_exec = SeccompCondition::new(
            2,
            SeccompCmpArgLen::Qword,
            SeccompCmpOp::MaskedEq(libc::PROT_EXEC as u64),
            0,
        )?;
        rules.insert(nr as i64, vec![SeccompRule::new(vec![no_exec])?]);
    }

    // …and fcntl, allowed only for memfd sealing plus the read-only F_GETFD
    // (`cmd` is argument index 1; multiple rules OR together). A renderer must
    // be able to seal its tile buffers, and Rust's std *debug* builds probe
    // fds with fcntl(F_GETFD) when an OwnedFd drops (debug_assert_fd_is_open)
    // - a pure query with nothing to escalate. Every *mutating* fcntl command
    // - F_DUPFD (fd fabrication), F_SETFL, locks - hits KillProcess. F_SETFD
    // is a special case handled below: permitted only to *set* close-on-exec,
    // never to clear it.
    let mut fcntl_allowed = Vec::new();
    // `F_GETFL` joins the query-only set: it reads a descriptor's flags and
    // grants nothing. Surfaced by TSYNC - tokio's socket threads probe it -
    // having been silently unfiltered before the filter covered all threads.
    let mut cmds = vec![libc::F_ADD_SEALS, libc::F_GET_SEALS, libc::F_GETFD, libc::F_GETFL];
    if allow_dup_fd {
        cmds.push(libc::F_DUPFD_CLOEXEC);
    }
    if allow_socket_fcntl {
        // The net stack toggles O_NONBLOCK (and friends) on its sockets from
        // its runtime threads; `F_SETFL` cannot fabricate descriptors or
        // escalate, it only changes I/O modes on fds the process already has.
        cmds.push(libc::F_SETFL);
    }
    for cmd in cmds {
        let is_cmd = SeccompCondition::new(1, SeccompCmpArgLen::Qword, SeccompCmpOp::Eq, cmd as u64)?;
        fcntl_allowed.push(SeccompRule::new(vec![is_cmd])?);
    }
    // `fcntl(F_SETFD, FD_CLOEXEC)` - permitted for every filter, not just the
    // fork server. musl issues it after any file open (its `std::fs` opens
    // with `O_CLOEXEC`, then redundantly re-sets `FD_CLOEXEC`) and after
    // `F_DUPFD_CLOEXEC`; glibc does neither. Gating it behind `allow_dup_fd`
    // killed every file open in a filesystem/device service under musl
    // (SIGSYS on syscall #72). Found on the musl CI row.
    let is_setfd = SeccompCondition::new(1, SeccompCmpArgLen::Qword, SeccompCmpOp::Eq, libc::F_SETFD as u64)?;
    let sets_cloexec = SeccompCondition::new(2, SeccompCmpArgLen::Qword, SeccompCmpOp::Eq, libc::FD_CLOEXEC as u64)?;
    fcntl_allowed.push(SeccompRule::new(vec![is_setfd, sets_cloexec])?);

    rules.insert(libc::SYS_fcntl as i64, fcntl_allowed);

    // …and `prctl`, argument-filtered to naming a thread. Spawning a thread
    // sets its name (tokio names its runtime and blocking-pool threads), so a
    // role that spawns one after lockdown dies without this. `PR_SET_NAME`
    // writes a 16-byte label on the calling thread and grants nothing; every
    // other prctl command still hits the default action. Skipped when the
    // caller already allows `prctl` outright (the fork server, which needs
    // `PR_SET_NO_NEW_PRIVS` for its children).
    if !allowed.contains(&libc::SYS_prctl) {
        let is_set_name =
            SeccompCondition::new(0, SeccompCmpArgLen::Qword, SeccompCmpOp::Eq, libc::PR_SET_NAME as u64)?;
        rules.insert(libc::SYS_prctl as i64, vec![SeccompRule::new(vec![is_set_name])?]);
    }

    // tgkill is permitted only to deliver SIGSYS (what `sigsys_handler` uses
    // to re-raise after logging). `sig` is argument index 2; any other signal
    // or target hits the Trap default.
    let sig_is_sigsys = SeccompCondition::new(2, SeccompCmpArgLen::Qword, SeccompCmpOp::Eq, libc::SIGSYS as u64)?;
    rules.insert(libc::SYS_tgkill as i64, vec![SeccompRule::new(vec![sig_is_sigsys])?]);

    // …and, for the fork server only, `clone` - argument-filtered to a plain
    // fork. With `clone3` ENOSYS'd ([`install_clone3_enosys`]), glibc's
    // `fork()` reaches the kernel as `clone` with flags in a register
    // (argument index 0). `MaskedEq(DANGEROUS, 0)` = "(flags & DANGEROUS) == 0":
    // a plain fork sets only SIGCHLD | CLONE_CHILD_SETTID | CLONE_CHILD_CLEARTID
    // (glibc and musl alike) and passes; CLONE_NEW*/CLONE_THREAD/CLONE_VM hit
    // the default action. Replaces the empty `clone` allow `allowed` would
    // otherwise produce.
    if fork_server {
        // Kernel CLONE_* bits (stable UAPI). Declared locally rather than via
        // `libc` so the mask does not depend on which constants a given `libc`
        // version happens to export.
        const CLONE_VM: u64 = 0x0000_0100;
        const CLONE_THREAD: u64 = 0x0001_0000;
        const CLONE_NEWTIME: u64 = 0x0000_0080;
        const CLONE_NEWNS: u64 = 0x0002_0000;
        const CLONE_NEWCGROUP: u64 = 0x0200_0000;
        const CLONE_NEWUTS: u64 = 0x0400_0000;
        const CLONE_NEWIPC: u64 = 0x0800_0000;
        const CLONE_NEWUSER: u64 = 0x1000_0000;
        const CLONE_NEWPID: u64 = 0x2000_0000;
        const CLONE_NEWNET: u64 = 0x4000_0000;
        const DANGEROUS: u64 = CLONE_VM
            | CLONE_THREAD
            | CLONE_NEWTIME
            | CLONE_NEWNS
            | CLONE_NEWCGROUP
            | CLONE_NEWUTS
            | CLONE_NEWIPC
            | CLONE_NEWUSER
            | CLONE_NEWPID
            | CLONE_NEWNET;
        let plain_fork = SeccompCondition::new(0, SeccompCmpArgLen::Qword, SeccompCmpOp::MaskedEq(DANGEROUS), 0)?;
        rules.insert(libc::SYS_clone as i64, vec![SeccompRule::new(vec![plain_fork])?]);
    }

    // Default is `Trap` (SECCOMP_RET_TRAP -> SIGSYS) rather than `KillProcess`
    // so a handler can name the blocked syscall on stderr, then re-raise
    // SIGSYS - the process still dies with the same signal (the selftest
    // probes assert that). The handler is installed before the filter applies,
    // so its own sigaction/getpid/gettid/tgkill/write run unfiltered here and
    // are on the allowlist for when it runs.
    install_sigsys_reporter();
    // …and the crash reporter: on SIGSEGV/ABRT/BUS/ILL/FPE, self-capture a
    // scrubbed, core-less crash report (see `install_crash_reporter`).
    install_crash_reporter();

    let filter = SeccompFilter::new(
        rules,
        SeccompAction::Trap,  // default & argument-mismatch: SIGSYS → sigsys_handler → re-raised
        SeccompAction::Allow, // matched: allow
        arch,
    )?;
    let program: BpfProgram = filter.try_into()?;
    // `apply_filter_all` (TSYNC) rather than per-thread: a role's library may
    // have created a thread before its lockdown (measured: Pango's GLib
    // worker), and a filter that missed it would leave one unconfined thread.
    apply_filter_all_threads(&program)?;
    Ok(())
}

/// Install a stacked pre-filter that turns `clone3` into `ENOSYS`, so glibc's
/// `fork()` retries with the register-based `clone` the main fork-server filter
/// argument-filters. `clone3` cannot be argument-filtered directly - it passes
/// its flags in a memory struct seccomp cannot dereference - so `ENOSYS`-ing it
/// is the only way to route fork onto a constrainable path. This is the standard
/// technique (Chromium, systemd) and relies on a fallback glibc has carried
/// since it started issuing `clone3`.
#[cfg(feature = "multi-process")]
fn install_clone3_enosys() -> Result<(), Box<dyn std::error::Error>> {
    use seccompiler::{apply_filter_all_threads, BpfProgram, SeccompAction, SeccompFilter, SeccompRule};
    use std::collections::BTreeMap;

    #[cfg(target_arch = "x86_64")]
    let arch = seccompiler::TargetArch::x86_64;
    #[cfg(target_arch = "aarch64")]
    let arch = seccompiler::TargetArch::aarch64;

    // One rule, any arguments: clone3 → the match action below.
    let mut rules: BTreeMap<i64, Vec<SeccompRule>> = BTreeMap::new();
    rules.insert(libc::SYS_clone3 as i64, Vec::new());

    let filter = SeccompFilter::new(
        rules,
        SeccompAction::Allow,                      // mismatch: defer to the main filter
        SeccompAction::Errno(libc::ENOSYS as u32), // match (clone3): ENOSYS, triggering fork's fallback
        arch,
    )?;
    let program: BpfProgram = filter.try_into()?;
    // `apply_filter_all` (TSYNC) rather than per-thread: a role's library may
    // have created a thread before its lockdown (measured: Pango's GLib
    // worker), and a filter that missed it would leave one unconfined thread.
    apply_filter_all_threads(&program)?;
    Ok(())
}

/// Install the SIGSYS reporter (SA_SIGINFO so the handler sees which syscall
/// trapped; SA_NODEFER so the re-raised SIGSYS is delivered synchronously
/// against the restored default disposition). Best-effort: if it cannot be
/// installed the `Trap` default still terminates the process on a violation -
/// it just does so without the diagnostic line.
#[cfg(feature = "multi-process")]
fn install_sigsys_reporter() {
    // SAFETY: zeroed sigaction is a valid empty handler; we then set the two
    // fields we need and register it for SIGSYS only.
    unsafe {
        let mut sa: libc::sigaction = std::mem::zeroed();
        sa.sa_sigaction = sigsys_handler as *const () as usize;
        sa.sa_flags = libc::SA_SIGINFO | libc::SA_NODEFER;
        libc::sigemptyset(&mut sa.sa_mask);
        libc::sigaction(libc::SIGSYS, &sa, std::ptr::null_mut());
    }
}

/// Which argument of the blocked syscall is a pathname pointer, if any - so
/// the SIGSYS report can name the file, not just the call. Numbers are
/// per-arch (the same numbers the seccomp filter matched on).
#[cfg(feature = "multi-process")]
fn path_arg_index(nr: i32) -> Option<usize> {
    #[cfg(target_arch = "x86_64")]
    match nr as i64 {
        libc::SYS_open
        | libc::SYS_stat
        | libc::SYS_lstat
        | libc::SYS_access
        | libc::SYS_readlink
        | libc::SYS_statfs
        | libc::SYS_truncate
        | libc::SYS_unlink
        | libc::SYS_chmod
        | libc::SYS_chown => Some(0),
        libc::SYS_openat
        | libc::SYS_openat2
        | libc::SYS_newfstatat
        | libc::SYS_readlinkat
        | libc::SYS_faccessat
        | libc::SYS_faccessat2
        | libc::SYS_unlinkat
        | libc::SYS_statx => Some(1),
        _ => None,
    }
    #[cfg(target_arch = "aarch64")]
    match nr as i64 {
        libc::SYS_openat
        | libc::SYS_openat2
        | libc::SYS_newfstatat
        | libc::SYS_readlinkat
        | libc::SYS_faccessat
        | libc::SYS_faccessat2
        | libc::SYS_unlinkat
        | libc::SYS_statx => Some(1),
        _ => None,
    }
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    {
        let _ = nr;
        None
    }
}

/// The `arg_index`-th syscall argument at the trap site, read from the signal
/// ucontext. `None` where the register layout is unknown.
#[cfg(feature = "multi-process")]
fn syscall_arg(ctx: *mut libc::c_void, arg_index: usize) -> Option<u64> {
    if ctx.is_null() {
        return None;
    }
    #[cfg(target_arch = "x86_64")]
    {
        // SAFETY: on a SIGSYS delivered with SA_SIGINFO the third handler
        // argument points at a kernel-filled ucontext_t.
        let uc = unsafe { &*(ctx as *const libc::ucontext_t) };
        let reg = match arg_index {
            0 => libc::REG_RDI,
            1 => libc::REG_RSI,
            2 => libc::REG_RDX,
            _ => return None,
        };
        Some(uc.uc_mcontext.gregs[reg as usize] as u64)
    }
    #[cfg(target_arch = "aarch64")]
    {
        // SAFETY: as above; aarch64 keeps syscall args in x0..x5.
        let uc = unsafe { &*(ctx as *const libc::ucontext_t) };
        if arg_index > 5 {
            return None;
        }
        Some(uc.uc_mcontext.regs[arg_index])
    }
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    {
        let _ = arg_index;
        None
    }
}

/// SIGSYS handler for `SECCOMP_RET_TRAP`: name the blocked syscall (and, for
/// path-taking calls, the path it was given), then terminate with SIGSYS
/// exactly as `KillProcess` would have.
#[cfg(feature = "multi-process")]
extern "C" fn sigsys_handler(_sig: libc::c_int, info: *mut libc::siginfo_t, ctx: *mut libc::c_void) {
    // `si_syscall` sits at byte offset 24 of `siginfo_t` on LP64 Linux - after
    // {si_signo, si_errno, si_code, pad} (16 bytes) and the `_call_addr`
    // pointer (8). Same layout on x86_64 and aarch64, the two arches this
    // crate builds seccomp for. A wrong read only mislabels the log line; it
    // cannot affect the termination below.
    let nr: i32 = if info.is_null() {
        -1
    } else {
        // SAFETY: `info` points at a kernel-filled siginfo_t at least 32 bytes
        // long; the read is unaligned-safe and within that.
        unsafe { std::ptr::read_unaligned((info as *const u8).add(24).cast::<i32>()) }
    };

    let mut buf = [0u8; 256];
    let mut len = 0usize;
    for &b in b"[sandbox] SIGSYS: blocked syscall #" {
        buf[len] = b;
        len += 1;
    }
    len += write_i32(&mut buf[len..], nr);

    // For a path-taking call, read the path the caller passed - the process is
    // dying anyway, and the pointer is the caller's own argument, so a plain
    // read is as safe as it gets in a handler. Bounded and sanitized.
    if let Some(ptr) = path_arg_index(nr).and_then(|idx| syscall_arg(ctx, idx)) {
        if ptr != 0 {
            for &b in b" (path \"" {
                buf[len] = b;
                len += 1;
            }
            let mut off = 0usize;
            while len < buf.len() - 20 && off < 160 {
                // SAFETY: reads the NUL-terminated string the trapped syscall
                // was about to consume, one byte at a time, bounded above.
                let byte = unsafe { std::ptr::read_volatile((ptr as *const u8).add(off)) };
                if byte == 0 {
                    break;
                }
                buf[len] = if (0x20..0x7f).contains(&byte) { byte } else { b'?' };
                len += 1;
                off += 1;
            }
            for &b in b"\")" {
                buf[len] = b;
                len += 1;
            }
        }
    }

    for &b in b" \xe2\x80\x94 terminating\n" {
        buf[len] = b;
        len += 1;
    }
    // SAFETY: fd 2 (stderr) is open; buf/len describe a valid initialized slice.
    unsafe {
        libc::write(2, buf.as_ptr().cast(), len);

        // Restore the default action and re-raise, so the process dies with
        // SIGSYS (the signal, and the exit semantics the probes check) rather
        // than returning from the trap and resuming the blocked call.
        let mut dfl: libc::sigaction = std::mem::zeroed();
        dfl.sa_sigaction = libc::SIG_DFL;
        libc::sigaction(libc::SIGSYS, &dfl, std::ptr::null_mut());
        let pid = libc::getpid();
        let tid = libc::syscall(libc::SYS_gettid) as libc::pid_t;
        libc::syscall(libc::SYS_tgkill, pid, tid, libc::SIGSYS);
        // Unreachable with SA_NODEFER (SIGSYS delivered synchronously above);
        // a belt-and-braces exit in case a future change masks it.
        libc::_exit(159);
    }
}

/// Crash reporting without a core dump and without `ptrace`.
#[cfg(feature = "multi-process")]
fn install_crash_reporter() {
    // Alternate signal stack, so a stack-overflow SIGSEGV can still run the
    // handler. `sigaltstack` is per-thread while `sigaction` is process-wide:
    // content processes are single-threaded so this thread suffices; broker
    // threads each call [`install_thread_crash_altstack`], else an overflow on
    // a worker thread runs the handler on the overflowed stack (process still
    // dies, but with no scrubbed report).
    static mut ALTSTACK: [u8; 16384] = [0; 16384];
    // SAFETY: a zeroed sigaction with the handler set; registered for the
    // synchronous crash signals only. `addr_of_mut!` avoids a reference to the
    // static. sigaltstack/sigaction are async-signal-safe and on every filter.
    unsafe {
        let ss = libc::stack_t {
            ss_sp: std::ptr::addr_of_mut!(ALTSTACK).cast(),
            ss_flags: 0,
            ss_size: 16384,
        };
        libc::sigaltstack(&ss, std::ptr::null_mut());

        let mut sa: libc::sigaction = std::mem::zeroed();
        sa.sa_sigaction = crash_handler as *const () as usize;
        sa.sa_flags = libc::SA_SIGINFO | libc::SA_NODEFER | libc::SA_ONSTACK;
        libc::sigemptyset(&mut sa.sa_mask);
        for sig in [libc::SIGSEGV, libc::SIGABRT, libc::SIGBUS, libc::SIGILL, libc::SIGFPE] {
            libc::sigaction(sig, &sa, std::ptr::null_mut());
        }
    }
}

/// Register an alternate signal stack for the current thread, so the
/// process-wide crash reporter can still run when *this* thread's own stack
/// overflows. [`install_crash_reporter`] arms the altstack only on the thread it
/// runs on; the broker is multithreaded, so each engine thread calls this at
/// startup. Backing storage lives in thread-local storage for the thread's
/// lifetime. A no-op-safe extra where the reporter was never installed.
#[cfg(feature = "multi-process")]
pub fn install_thread_crash_altstack() {
    thread_local! {
        static ALT: std::cell::UnsafeCell<[u8; 16384]> =
            const { std::cell::UnsafeCell::new([0u8; 16384]) };
    }
    ALT.with(|cell| {
        // SAFETY: the TLS buffer lives for this thread's lifetime and is handed
        // only to sigaltstack; we never otherwise alias it. sigaltstack is
        // async-signal-safe and on every filter.
        let ss = libc::stack_t {
            ss_sp: cell.get().cast(),
            ss_flags: 0,
            ss_size: 16384,
        };
        unsafe {
            libc::sigaltstack(&ss, std::ptr::null_mut());
        }
    });
}

/// Signal handler for the crash signals: emit a scrubbed one-line report, then
/// restore the default disposition and return so the fault re-triggers and kills
/// us with the original signal. Async-signal-safe: a stack buffer, hand-rolled
/// integer formatting, one `write`, one `sigaction`. No allocation, no locks.
#[cfg(feature = "multi-process")]
extern "C" fn crash_handler(sig: libc::c_int, info: *mut libc::siginfo_t, _ctx: *mut libc::c_void) {
    // The faulting address sits at byte offset 16 of `siginfo_t` on LP64 Linux
    // (`_sigfault.si_addr`, after {si_signo, si_errno, si_code, pad}). Same layout
    // on x86_64 and aarch64. Zero if the kernel gave us no siginfo.
    let addr: usize = if info.is_null() {
        0
    } else {
        // SAFETY: kernel-filled siginfo is at least 24 bytes; unaligned-safe read.
        unsafe { std::ptr::read_unaligned((info as *const u8).add(16).cast::<usize>()) }
    };

    let mut buf = [0u8; 128];
    let mut len = 0usize;
    // Inline byte copies (no closure, so the slice borrows below stay free), the
    // same shape as the SIGSYS reporter.
    for chunk in [b"[crash] ".as_slice(), signal_name(sig), b" (#".as_slice()] {
        for &b in chunk {
            buf[len] = b;
            len += 1;
        }
    }
    len += write_i32(&mut buf[len..], sig);
    for &b in b") at fault address " {
        buf[len] = b;
        len += 1;
    }
    len += write_hex_usize(&mut buf[len..], addr);
    // em-dash matches the SIGSYS reporter's style
    for &b in b" \xe2\x80\x94 terminating (no core, self-captured)\n" {
        buf[len] = b;
        len += 1;
    }

    // SAFETY: fd 2 is open; buf/len describe an initialized slice. Then restore
    // SIG_DFL and return - the fault re-executes and the default action kills us.
    unsafe {
        libc::write(2, buf.as_ptr().cast(), len);
        let mut dfl: libc::sigaction = std::mem::zeroed();
        dfl.sa_sigaction = libc::SIG_DFL;
        libc::sigaction(sig, &dfl, std::ptr::null_mut());
    }
}

/// The crash signals' names as static bytes (async-signal-safe: no formatting).
#[cfg(feature = "multi-process")]
fn signal_name(sig: libc::c_int) -> &'static [u8] {
    match sig {
        libc::SIGSEGV => b"SIGSEGV",
        libc::SIGABRT => b"SIGABRT",
        libc::SIGBUS => b"SIGBUS",
        libc::SIGILL => b"SIGILL",
        libc::SIGFPE => b"SIGFPE",
        _ => b"signal",
    }
}

/// Async-signal-safe `0x`-prefixed hex formatter for a pointer-sized value.
/// Writes into `out` and returns the byte count. No allocation.
#[cfg(feature = "multi-process")]
fn write_hex_usize(out: &mut [u8], mut v: usize) -> usize {
    out[0] = b'0';
    out[1] = b'x';
    let mut len = 2usize;
    if v == 0 {
        out[len] = b'0';
        return len + 1;
    }
    let mut digits = [0u8; 16];
    let mut d = 0usize;
    while v > 0 {
        let nybble = (v & 0xf) as u8;
        digits[d] = if nybble < 10 {
            b'0' + nybble
        } else {
            b'a' + (nybble - 10)
        };
        v >>= 4;
        d += 1;
    }
    while d > 0 {
        d -= 1;
        out[len] = digits[d];
        len += 1;
    }
    len
}

/// Async-signal-safe decimal formatter for the SIGSYS reporter: writes `v`
/// (handling a negative) into `out` and returns the byte count. No allocation.
#[cfg(feature = "multi-process")]
fn write_i32(out: &mut [u8], v: i32) -> usize {
    let mut n = v as i64;
    let neg = n < 0;
    if neg {
        n = -n;
    }
    let mut digits = [0u8; 10];
    let mut d = 0usize;
    if n == 0 {
        digits[d] = b'0';
        d += 1;
    }
    while n > 0 {
        digits[d] = b'0' + (n % 10) as u8;
        n /= 10;
        d += 1;
    }
    let mut len = 0usize;
    if neg {
        out[len] = b'-';
        len += 1;
    }
    while d > 0 {
        d -= 1;
        out[len] = digits[d];
        len += 1;
    }
    len
}

/// Resource ceilings the engine imposes on a child at spawn time. seccomp caps
/// *what* syscalls a child may run; this caps *how much* it may consume, so a
/// compromised child cannot exhaust host memory or fd tables. rlimits can only
/// ever be lowered, never raised, so the child cannot undo them.
#[cfg(feature = "multi-process")]
pub fn apply_child_rlimits() -> std::io::Result<()> {
    // Bound the heap, not the address space: `RLIMIT_DATA` caps committed
    // writable anonymous memory (brk + writable mmap) and since Linux 4.7 does
    // not count PROT_NONE reservations, so V8's ~4 GiB virtual cage still
    // fits while a runaway allocation aborts only that process. `RLIMIT_AS`
    // would kill V8 at init for reserving space it never commits. A cgroup
    // `memory.max` bounds true RSS; this rlimit is the cheap approximation
    // (see the architecture doc).
    set_rlimit(libc::RLIMIT_DATA, 512 * 1024 * 1024)?;
    // A generous virtual ceiling on top: high enough to clear a JIT's cage,
    // low enough to catch a runaway reserving absurd address space.
    set_rlimit(libc::RLIMIT_AS, 16 * 1024 * 1024 * 1024)?;
    // A child needs only a handful of fds (its IPC socket + std streams).
    set_rlimit(libc::RLIMIT_NOFILE, 128)?;
    // No core dumps - a crash must not spill page contents (cookies, tokens).
    set_rlimit(libc::RLIMIT_CORE, 0)?;
    // Deprioritize: content processes should yield to the trusted engine/UI, so
    // a compromised child spinning in a busy loop can't starve them of CPU. A
    // hard RLIMIT_CPU is unusable here - it counts *cumulative* CPU time and
    // would eventually kill a legitimately long-lived renderer - so we lower
    // scheduling priority instead. Raising the nice value is always permitted
    // and needs no privilege, so a child can't undo it either.
    set_priority(10)?;
    Ok(())
}

/// Move the calling process into fresh, empty namespaces when `enable` is set
/// (content processes and the engine-spawned services); a no-op otherwise (the
/// net component, the one role that must keep the host network).
#[cfg(feature = "multi-process")]
pub fn isolate_namespaces(mode: crate::NamespaceIsolation) -> std::io::Result<()> {
    use crate::NamespaceIsolation;

    if matches!(mode, NamespaceIsolation::None) {
        return Ok(());
    }
    let flags = libc::CLONE_NEWUSER | libc::CLONE_NEWNET | libc::CLONE_NEWIPC | libc::CLONE_NEWUTS;
    // `Full` also requests a PID namespace. `unshare(CLONE_NEWPID)` does not
    // move the caller - it places the caller's *future children* in a new PID
    // namespace - but it is NOT inert for a non-forking service: a process that
    // has unshared its PID namespace can no longer create *threads* (a thread
    // must share its creator's PID namespace, and new tasks now go to the new
    // one, so `pthread_create` fails `EINVAL` - GLib escalates that to a fatal
    // abort, measured with Pango). Roles whose libraries must thread use
    // `NoPidNamespace` and give up the PID isolation of anything they fork -
    // which such roles don't do. Best-effort and tried *first* as one combined
    // `unshare` (creating the PID namespace unprivileged requires pairing it
    // with the user namespace in the same call); a kernel that refuses
    // `CLONE_NEWPID` falls back to the network isolation alone rather than
    // failing the spawn.
    // SAFETY: unshare with valid flags; affects only the calling process.
    if matches!(mode, NamespaceIsolation::Full) && unsafe { libc::unshare(flags | libc::CLONE_NEWPID) } == 0 {
        return Ok(());
    }
    // NoPidNamespace, or the PID attempt was refused: the load-bearing network
    // isolation alone. `unshare` is all-or-nothing, so a failed attempt above
    // changed nothing.
    if unsafe { libc::unshare(flags) } < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

/// Mark the calling process non-dumpable, closing the *inbound* debugging
/// surface.
pub fn deny_debugger_attach() {
    // SAFETY: PR_SET_DUMPABLE takes one value argument and affects only the
    // calling process.
    if unsafe { libc::prctl(libc::PR_SET_DUMPABLE, 0) } < 0 {
        // Not fatal, unlike seccomp: hardening against other host software,
        // not the boundary containing a compromised renderer.
        eprintln!(
            "[sandbox] warning: could not clear dumpable flag: {}",
            std::io::Error::last_os_error()
        );
    }
}

/// Lower the calling process's scheduling priority (higher nice = lower
/// priority). Async-signal-safe (a single syscall), so usable pre-exec.
#[cfg(feature = "multi-process")]
fn set_priority(nice: libc::c_int) -> std::io::Result<()> {
    // SAFETY: PRIO_PROCESS with pid 0 targets the calling process.
    if unsafe { libc::setpriority(libc::PRIO_PROCESS, 0, nice) } < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

/// The first argument of `setrlimit(2)` is libc-dependent: glibc exposes
/// `__rlimit_resource_t`, musl a plain `c_int`; naming either directly breaks
/// the build on the other libc.
#[cfg(all(feature = "multi-process", target_env = "gnu"))]
type RlimitResource = libc::__rlimit_resource_t;
#[cfg(all(feature = "multi-process", not(target_env = "gnu")))]
type RlimitResource = libc::c_int;

#[cfg(feature = "multi-process")]
fn set_rlimit(resource: RlimitResource, limit: libc::rlim_t) -> std::io::Result<()> {
    let rl = libc::rlimit {
        rlim_cur: limit,
        rlim_max: limit,
    };
    // SAFETY: valid resource id and a valid rlimit pointer.
    if unsafe { libc::setrlimit(resource, &rl) } < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}
