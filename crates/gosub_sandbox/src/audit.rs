//! The escape audit: what a confined process can still reach, measured from
//! inside it after lockdown. Each check attempts one operation an attacker
//! holding this process would try - open a file, a socket, fork, signal the
//! broker - and records the outcome. A seccomp trap is caught here and turned
//! into an errno, so the audit survives its own attempts; the report is what
//! the broker (or a test) compares against the role's expectations.
//!
//! The per-primitive probes in `selftest` prove single filter rules die the
//! way they should. This runs in the *real* child - after the real spawn,
//! inheritance and lockdown - and asks the whole question at once.

use serde::{Deserialize, Serialize};
#[cfg(target_os = "linux")]
use std::sync::atomic::{AtomicBool, Ordering};

/// Which lockdown the audited process is under; decides what is expected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role {
    Renderer,
    ForkServer,
    Net,
    Decoder,
    Vault,
    /// A filesystem service scoped to its own directory.
    Storage,
}

/// What an attempted operation came to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Outcome {
    /// seccomp trapped the syscall (SIGSYS): the filter denied it.
    Trapped,
    /// The kernel refused it (Landlock, DAC, a namespace): the errno.
    Errno(i32),
    /// It worked; the detail says what was reached.
    Allowed(String),
}

impl Outcome {
    pub fn denied(&self) -> bool {
        !matches!(self, Outcome::Allowed(_))
    }
}

/// What the role's design says should happen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Expect {
    Denied,
    Allowed,
    /// Recorded for the reader, not judged (inherited fds, environment).
    Info,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditItem {
    pub check: String,
    pub outcome: Outcome,
    pub expect: Expect,
}

impl AuditItem {
    /// Whether the outcome contradicts the expectation.
    pub fn violated(&self) -> bool {
        match self.expect {
            Expect::Denied => !self.outcome.denied(),
            Expect::Allowed => self.outcome.denied(),
            Expect::Info => false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditReport {
    pub role: Role,
    pub pid: u32,
    pub items: Vec<AuditItem>,
}

impl AuditReport {
    pub fn violations(&self) -> Vec<&AuditItem> {
        self.items.iter().filter(|i| i.violated()).collect()
    }

    /// One line per check, for a terminal.
    pub fn render(&self) -> String {
        let mut out = format!("{:?} (pid {})\n", self.role, self.pid);
        for item in &self.items {
            let outcome = match &item.outcome {
                Outcome::Trapped => "trapped (SIGSYS)".to_string(),
                Outcome::Errno(e) => format!("errno {e} ({})", std::io::Error::from_raw_os_error(*e)),
                Outcome::Allowed(detail) if detail.is_empty() => "ALLOWED".to_string(),
                Outcome::Allowed(detail) => format!("ALLOWED: {detail}"),
            };
            let mark = match (item.expect, item.violated()) {
                (Expect::Info, _) => "  ",
                (_, false) => "ok",
                (_, true) => "!!",
            };
            out.push_str(&format!("  {mark} {:<38} {outcome}\n", item.check));
        }
        out
    }
}

#[cfg(target_os = "linux")]
mod runner {
    use super::*;

    /// Set while a SIGSYS from one of the audit's own attempts is being turned
    /// into an errno.
    static TRAPPED: AtomicBool = AtomicBool::new(false);

    /// SIGSYS handler for the audit: mark the trap and make the syscall return
    /// `-EPERM` instead of killing the process. The reporter that normally owns
    /// SIGSYS is put back once the audit is over.
    extern "C" fn audit_sigsys(_sig: libc::c_int, _info: *mut libc::siginfo_t, ctx: *mut libc::c_void) {
        TRAPPED.store(true, Ordering::SeqCst);
        if ctx.is_null() {
            return;
        }
        // SAFETY: the kernel hands a SA_SIGINFO handler a `ucontext_t`; the
        // register written is the syscall return slot on this architecture.
        unsafe {
            let uc = ctx.cast::<libc::ucontext_t>();
            #[cfg(target_arch = "x86_64")]
            {
                (*uc).uc_mcontext.gregs[libc::REG_RAX as usize] = -(libc::EPERM as i64);
            }
            #[cfg(target_arch = "aarch64")]
            {
                (*uc).uc_mcontext.regs[0] = (-(libc::EPERM as i64)) as u64;
            }
        }
    }

    /// Run `attempt` with SIGSYS caught; `Err(Trapped)` when the filter denied
    /// what it tried, else the attempt's own verdict.
    fn intercept(attempt: impl FnOnce() -> Outcome) -> Outcome {
        // SAFETY: installing a handler with a valid sigaction; restored below.
        let previous = unsafe {
            let mut sa: libc::sigaction = std::mem::zeroed();
            sa.sa_sigaction = audit_sigsys as *const () as usize;
            sa.sa_flags = libc::SA_SIGINFO | libc::SA_NODEFER;
            libc::sigemptyset(&mut sa.sa_mask);
            let mut previous: libc::sigaction = std::mem::zeroed();
            libc::sigaction(libc::SIGSYS, &sa, &mut previous);
            previous
        };
        TRAPPED.store(false, Ordering::SeqCst);
        let outcome = attempt();
        let trapped = TRAPPED.swap(false, Ordering::SeqCst);
        // SAFETY: restoring the action captured above.
        unsafe { libc::sigaction(libc::SIGSYS, &previous, std::ptr::null_mut()) };
        if trapped {
            Outcome::Trapped
        } else {
            outcome
        }
    }

    fn errno() -> i32 {
        std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
    }

    fn cstr(s: &str) -> std::ffi::CString {
        std::ffi::CString::new(s).unwrap_or_default()
    }

    /// `open(path)` for reading; closes it again when it worked.
    fn try_open(path: &str, flags: libc::c_int) -> Outcome {
        let p = cstr(path);
        // SAFETY: a NUL-terminated path; the fd is closed right away.
        let fd = unsafe { libc::open(p.as_ptr(), flags | libc::O_CLOEXEC, 0o600) };
        if fd < 0 {
            return Outcome::Errno(errno());
        }
        // SAFETY: fd is ours.
        unsafe { libc::close(fd) };
        Outcome::Allowed(String::new())
    }

    fn try_socket(domain: libc::c_int, ty: libc::c_int) -> Outcome {
        // SAFETY: plain syscall; a returned fd is closed right away.
        let fd = unsafe { libc::socket(domain, ty | libc::SOCK_CLOEXEC, 0) };
        if fd < 0 {
            return Outcome::Errno(errno());
        }
        unsafe { libc::close(fd) };
        Outcome::Allowed(String::new())
    }

    fn rc(r: libc::c_long) -> Outcome {
        if r < 0 {
            Outcome::Errno(errno())
        } else {
            Outcome::Allowed(String::new())
        }
    }

    /// Fork, and have the child exit at once. Not reaped here: `wait4` is not on
    /// every allowlist, and a zombie until this process ends is no harm.
    fn try_fork() -> Outcome {
        // SAFETY: the child does nothing but `_exit`.
        let pid = unsafe { libc::fork() };
        if pid < 0 {
            return Outcome::Errno(errno());
        }
        if pid == 0 {
            unsafe { libc::_exit(0) };
        }
        Outcome::Allowed(format!("child pid {pid}"))
    }

    fn try_exec() -> Outcome {
        // With `fork` denied this would replace the audited process, so it only
        // runs where a fork worked first; `execve` of a file that does not exist
        // still has to pass the filter to fail with ENOENT.
        let path = cstr("/nonexistent/gosub-audit");
        let argv = [path.as_ptr(), std::ptr::null()];
        let envp = [std::ptr::null()];
        // SAFETY: valid NUL-terminated arrays; ENOENT or a trap, never a replacement.
        let r = unsafe { libc::execve(path.as_ptr(), argv.as_ptr(), envp.as_ptr()) };
        match rc(r as libc::c_long) {
            Outcome::Errno(e) if e == libc::ENOENT => Outcome::Allowed("execve reached the kernel".into()),
            other => other,
        }
    }

    fn try_exec_memory() -> Outcome {
        // SAFETY: an anonymous mapping, unmapped again.
        unsafe {
            let p = libc::mmap(
                std::ptr::null_mut(),
                4096,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
                -1,
                0,
            );
            if p == libc::MAP_FAILED {
                return Outcome::Errno(errno());
            }
            let r = libc::mprotect(p, 4096, libc::PROT_READ | libc::PROT_EXEC);
            let out = rc(r as libc::c_long);
            libc::munmap(p, 4096);
            out
        }
    }

    /// The descriptors this process holds, by number and kind.
    fn open_fds() -> String {
        let mut out = Vec::new();
        for fd in 0..256 {
            // SAFETY: F_GETFD on a possibly-closed fd is harmless.
            if unsafe { libc::fcntl(fd, libc::F_GETFD) } < 0 {
                continue;
            }
            let mut st: libc::stat = unsafe { std::mem::zeroed() };
            // SAFETY: fstat into a zeroed struct.
            let kind = if unsafe { libc::fstat(fd, &mut st) } == 0 {
                match st.st_mode & libc::S_IFMT {
                    libc::S_IFSOCK => "sock",
                    libc::S_IFIFO => "pipe",
                    libc::S_IFCHR => "chr",
                    libc::S_IFREG => "file",
                    libc::S_IFDIR => "dir",
                    _ => "other",
                }
            } else {
                "?"
            };
            out.push(format!("{fd}:{kind}"));
        }
        out.join(" ")
    }

    fn env_keys() -> String {
        let mut keys: Vec<String> = std::env::vars_os()
            .map(|(k, _)| k.to_string_lossy().into_owned())
            .collect();
        keys.sort();
        keys.join(" ")
    }

    fn check(items: &mut Vec<AuditItem>, name: &str, expect: Expect, attempt: &dyn Fn() -> Outcome) {
        items.push(AuditItem {
            check: name.to_string(),
            outcome: intercept(attempt),
            expect,
        });
    }

    /// Run the audit for `role`. `own_paths` are what a scoped service may
    /// touch (its directory); everything else on the filesystem is expected out
    /// of reach.
    pub fn run(role: Role, own_paths: &[std::path::PathBuf]) -> AuditReport {
        use Expect::{Allowed, Denied, Info};
        let mut items: Vec<AuditItem> = Vec::new();
        let files = matches!(role, Role::Net | Role::Storage);
        let network = role == Role::Net;
        let forks = role == Role::ForkServer;
        let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());

        // Files: only a scoped service reaches its own paths; nobody reaches the rest.
        check(&mut items, "open /etc/passwd", Denied, &|| {
            try_open("/etc/passwd", libc::O_RDONLY)
        });
        check(&mut items, "open $HOME", Denied, &|| {
            try_open(&home, libc::O_RDONLY | libc::O_DIRECTORY)
        });
        check(&mut items, "open /proc/self/maps", Denied, &|| {
            try_open("/proc/self/maps", libc::O_RDONLY)
        });
        check(&mut items, "open /proc/1/status", Denied, &|| {
            try_open("/proc/1/status", libc::O_RDONLY)
        });
        check(&mut items, "create /tmp file", Denied, &|| {
            try_open(
                &format!("/tmp/gosub-audit-{}", std::process::id()),
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL,
            )
        });
        if files {
            check(
                &mut items,
                "open /etc/resolv.conf",
                if network { Allowed } else { Denied },
                &|| try_open("/etc/resolv.conf", libc::O_RDONLY),
            );
        }
        for path in own_paths {
            let p = path.to_string_lossy().into_owned();
            check(&mut items, &format!("open own {p}"), Allowed, &|| {
                try_open(&p, libc::O_RDONLY)
            });
            // An unnamed file in the directory: proves write access, leaves nothing.
            check(&mut items, &format!("create in own {p}"), Allowed, &|| {
                try_open(&p, libc::O_TMPFILE | libc::O_WRONLY)
            });
        }

        // Network: internet families for the net role, nothing for anyone else;
        // never the unix/netlink families that reach the session.
        check(
            &mut items,
            "socket AF_INET",
            if network { Allowed } else { Denied },
            &|| try_socket(libc::AF_INET, libc::SOCK_STREAM),
        );
        check(&mut items, "socket AF_UNIX", Denied, &|| {
            try_socket(libc::AF_UNIX, libc::SOCK_STREAM)
        });
        check(&mut items, "socket AF_NETLINK", Denied, &|| {
            try_socket(libc::AF_NETLINK, libc::SOCK_RAW)
        });

        // Processes: only the fork server forks; nobody execs.
        check(&mut items, "fork", if forks { Allowed } else { Denied }, &try_fork);
        if forks {
            // Where fork works, exec in the parent would replace us: test it in a child.
            // SAFETY: the child exits with the verdict.
            let pid = unsafe { libc::fork() };
            if pid == 0 {
                let code = match intercept(try_exec) {
                    Outcome::Allowed(_) => 3,
                    _ => 0,
                };
                unsafe { libc::_exit(code) };
            }
            let mut status = 0;
            unsafe { libc::waitpid(pid, &mut status, 0) };
            let allowed = libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == 3;
            items.push(AuditItem {
                check: "execve".into(),
                outcome: if allowed {
                    Outcome::Allowed("execve reached the kernel".into())
                } else {
                    Outcome::Trapped
                },
                expect: Denied,
            });
        } else {
            check(&mut items, "execve", Denied, &try_exec);
        }
        check(&mut items, "unshare(CLONE_NEWUSER)", Denied, &|| {
            // SAFETY: plain syscall.
            rc(unsafe { libc::unshare(libc::CLONE_NEWUSER) } as libc::c_long)
        });
        check(&mut items, "kill(parent, 0)", Denied, &|| {
            // SAFETY: signal 0 delivers nothing.
            rc(unsafe { libc::kill(libc::getppid(), 0) } as libc::c_long)
        });
        check(&mut items, "tgkill(1, 1, 0)", Denied, &|| {
            // SAFETY: signal 0 delivers nothing; pid 1 is EPERM anyway.
            rc(unsafe { libc::syscall(libc::SYS_tgkill, 1, 1, 0) })
        });
        check(&mut items, "ptrace(TRACEME)", Denied, &|| {
            // SAFETY: with no tracer attached this only marks the process.
            rc(
                unsafe { libc::ptrace(libc::PTRACE_TRACEME, 0, std::ptr::null_mut::<libc::c_void>(), 0) }
                    as libc::c_long,
            )
        });
        check(&mut items, "prlimit(pid 1)", Denied, &|| {
            let mut lim: libc::rlimit = unsafe { std::mem::zeroed() };
            // SAFETY: a query on another pid; out-struct valid.
            rc(unsafe { libc::prlimit(1, libc::RLIMIT_NOFILE, std::ptr::null(), &mut lim) } as libc::c_long)
        });

        // Memory: no executable pages, ever.
        check(&mut items, "mprotect PROT_EXEC", Denied, &try_exec_memory);

        // For the reader: what the process inherited.
        items.push(AuditItem {
            check: "open fds".into(),
            outcome: Outcome::Allowed(open_fds()),
            expect: Info,
        });
        items.push(AuditItem {
            check: "environment".into(),
            outcome: Outcome::Allowed(env_keys()),
            expect: Info,
        });

        AuditReport {
            role,
            pid: std::process::id(),
            items,
        }
    }
}

#[cfg(target_os = "linux")]
pub use runner::run;
