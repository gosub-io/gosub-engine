//! Sandbox self-tests, spawned by the integration suite (never by the demo).
//!
//! Each probe applies the renderer lockdown and then attempts one operation,
//! letting the test harness observe the outcome from the *outside*: a forbidden
//! syscall is a fatal `SIGSYS` (seccomp `KillProcess`), while an allowed program
//! exits cleanly. We can't assert this from inside a `#[cfg(test)]` unit test
//! because the filter is irreversible and would kill the test runner itself -
//! so enforcement is checked here, in a throwaway child process.

// Probe code, compiled into the throwaway `sandbox-probe` binary and never into
// an engine process. Here a panic is the *reporting channel*: a probe whose
// precondition fails must die loudly so the harness sees a non-zero status,
// rather than degrade into a pass that verifies nothing. The engine's
// no-panic rule still governs everything the engine itself links.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

/// Fork a child that optionally marks itself non-dumpable, then try to
/// `PTRACE_ATTACH` to it from here (its parent, so Yama permits the attempt).
#[cfg(all(feature = "multi-process", target_os = "linux"))]
fn attach_refused(protect: bool) -> Option<i32> {
    let mut ready = [0 as libc::c_int; 2];
    assert_eq!(unsafe { libc::pipe(ready.as_mut_ptr()) }, 0, "pipe");
    let (rd, wr) = (ready[0], ready[1]);

    // SAFETY: single-threaded at this point, so the child may run normal code.
    let pid = unsafe { libc::fork() };
    assert!(pid >= 0, "fork");
    if pid == 0 {
        unsafe { libc::close(rd) };
        if protect {
            crate::deny_debugger_attach();
        }
        // Signal that the flag is set *before* parking, so the parent never
        // races ahead and attaches to a child that hasn't protected itself yet.
        unsafe {
            libc::write(wr, [1u8].as_ptr().cast(), 1);
            loop {
                libc::pause();
            }
        }
    }

    unsafe { libc::close(wr) };
    let mut byte = [0u8; 1];
    assert_eq!(
        unsafe { libc::read(rd, byte.as_mut_ptr().cast(), 1) },
        1,
        "child never signalled"
    );
    unsafe { libc::close(rd) };

    // SAFETY: PTRACE_ATTACH takes the target pid; the addr/data args are unused.
    let rc = unsafe {
        libc::ptrace(
            libc::PTRACE_ATTACH,
            pid,
            std::ptr::null_mut::<libc::c_void>(),
            std::ptr::null_mut::<libc::c_void>(),
        )
    };
    let outcome = if rc == -1 {
        Some(std::io::Error::last_os_error().raw_os_error().unwrap_or(0))
    } else {
        // A successful attach stops the tracee; reap that stop before killing.
        unsafe { libc::waitpid(pid, std::ptr::null_mut(), 0) };
        None
    };

    unsafe {
        libc::kill(pid, libc::SIGKILL);
        libc::waitpid(pid, std::ptr::null_mut(), 0);
    }
    outcome
}

/// Every probe compiled into *this* binary, for this platform.
pub const PROBES: &[&str] = &[
    #[cfg(target_os = "linux")]
    "baseline",
    #[cfg(target_os = "linux")]
    "mprotect-exec",
    #[cfg(target_os = "linux")]
    "socket",
    #[cfg(target_os = "linux")]
    "memfd-seal",
    #[cfg(target_os = "linux")]
    "fcntl-dupfd",
    #[cfg(target_os = "linux")]
    "ring",
    #[cfg(target_os = "linux")]
    "netns",
    #[cfg(target_os = "linux")]
    "pidns",
    #[cfg(target_os = "linux")]
    "no-ptrace",
    #[cfg(target_os = "linux")]
    "forkserver-can-fork",
    #[cfg(target_os = "linux")]
    "forkserver-canary-gap",
    #[cfg(target_os = "linux")]
    "forkserver-no-exec",
    #[cfg(target_os = "linux")]
    "forkserver-no-socket",
    #[cfg(target_os = "linux")]
    "forkserver-no-newuser-clone",
    #[cfg(target_os = "linux")]
    "service-fs-openat",
    #[cfg(target_os = "linux")]
    "service-fs-no-socket",
    #[cfg(target_os = "linux")]
    "service-device-ioctl",
    #[cfg(target_os = "linux")]
    "service-landlock",
    #[cfg(target_os = "linux")]
    "broker-landlock",
    #[cfg(target_os = "linux")]
    "broker-seccomp",
    #[cfg(target_os = "linux")]
    "broker-seccomp-mount",
    #[cfg(target_os = "linux")]
    "cgroup-memory-limit",
    #[cfg(target_os = "linux")]
    "crash-report",
    #[cfg(target_os = "macos")]
    "seatbelt-file",
    #[cfg(target_os = "macos")]
    "seatbelt-network",
    #[cfg(target_os = "macos")]
    "seatbelt-exec",
    #[cfg(target_os = "macos")]
    "seatbelt-net-role-keeps-network",
    #[cfg(target_os = "macos")]
    "seatbelt-baseline",
    #[cfg(target_os = "macos")]
    "seatbelt-file-write",
    #[cfg(target_os = "macos")]
    "seatbelt-fork",
    #[cfg(target_os = "macos")]
    "seatbelt-signal-other",
    #[cfg(target_os = "macos")]
    "seatbelt-sysctl",
    #[cfg(target_os = "macos")]
    "seatbelt-mach-lookup",
    #[cfg(target_os = "macos")]
    "seatbelt-service-scope",
    #[cfg(target_os = "macos")]
    "rlimits",
    #[cfg(target_os = "macos")]
    "ptrace-deny-accepted",
    #[cfg(target_os = "windows")]
    "mitigation-baseline",
    #[cfg(target_os = "windows")]
    "mitigation-dynamic-code",
    #[cfg(target_os = "windows")]
    "mitigation-child-process",
    #[cfg(target_os = "windows")]
    "mitigation-policies-readback",
    #[cfg(target_os = "windows")]
    "low-integrity",
    #[cfg(target_os = "windows")]
    "job-memory-limit",
    #[cfg(target_os = "windows")]
    "restricted-token",
];

/// Outcome codes for the Windows probes, mirroring the macOS set.
///
/// Windows mitigation policies behave like Seatbelt rather than seccomp: a
/// denied operation returns an error and the process keeps running, so these
/// probes must report what they observed instead of dying in a way the harness
/// could read from outside.
#[cfg(target_os = "windows")]
mod wcode {
    pub const CONTROL_FAILED: i32 = 90;
    pub const NOT_DENIED: i32 = 91;
    pub const WRONG_VALUE: i32 = 93;
}

/// How many privileges a token carries. The first `DWORD` of
/// `TOKEN_PRIVILEGES` is the count, so a modest buffer reads it even when the
/// full list would not fit.
#[cfg(target_os = "windows")]
fn privilege_count(token: windows_sys::Win32::Foundation::HANDLE) -> Option<u32> {
    use windows_sys::Win32::Security::{GetTokenInformation, TokenPrivileges};
    let mut buf = [0u8; 4096];
    let mut needed = 0u32;
    // SAFETY: correctly sized buffer with its length and an out-param.
    let ok = unsafe {
        GetTokenInformation(
            token,
            TokenPrivileges,
            buf.as_mut_ptr().cast(),
            buf.len() as u32,
            &mut needed,
        )
    };
    if ok == 0 {
        return None;
    }
    Some(u32::from_ne_bytes([buf[0], buf[1], buf[2], buf[3]]))
}

/// Try to allocate memory that is writable *and* executable - the allocation a
/// memory-corruption exploit needs in order to run injected code, and exactly
/// what `ProhibitDynamicCode` exists to refuse.
#[cfg(target_os = "windows")]
fn try_alloc_executable() -> bool {
    use windows_sys::Win32::System::Memory::{
        VirtualAlloc, VirtualFree, MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_EXECUTE_READWRITE,
    };
    // SAFETY: a standard anonymous reserve+commit; the pointer is freed below.
    unsafe {
        let p = VirtualAlloc(std::ptr::null(), 4096, MEM_COMMIT | MEM_RESERVE, PAGE_EXECUTE_READWRITE);
        if p.is_null() {
            return false;
        }
        VirtualFree(p, 0, MEM_RELEASE);
        true
    }
}

/// The Windows probe set: process mitigation policy enforcement.
#[cfg(target_os = "windows")]
fn run_windows_probe(probe: &str) {
    use windows_sys::Win32::System::Threading::{
        ProcessChildProcessPolicy, ProcessDynamicCodePolicy, ProcessExtensionPointDisablePolicy,
    };

    match probe {
        // The control for every denial below: ordinary work must still run.
        // A policy set that broke the component would satisfy each negative
        // probe while shipping a renderer that cannot render.
        "mitigation-baseline" => {
            crate::lock_down_renderer();
            let buf: Vec<u8> = (0..4096u32).map(|i| (i % 251) as u8).collect();
            let sum: u64 = buf.iter().map(|b| *b as u64).sum();
            eprintln!("[selftest] baseline computed {sum} under the policies");
            std::process::exit(0);
        }

        // W^X. The direct counterpart of the seccomp `PROT_EXEC` argument
        // filter, and the step most exploit chains need in order to execute
        // injected code.
        "mitigation-dynamic-code" => {
            if !try_alloc_executable() {
                std::process::exit(wcode::CONTROL_FAILED);
            }
            crate::lock_down_renderer();
            if try_alloc_executable() {
                std::process::exit(wcode::NOT_DENIED);
            }
            std::process::exit(0);
        }

        // No new programs - the analogue of `execve`/`clone` being absent from
        // the seccomp allowlist.
        "mitigation-child-process" => {
            if std::process::Command::new("cmd.exe")
                .args(["/C", "exit"])
                .status()
                .is_err()
            {
                std::process::exit(wcode::CONTROL_FAILED);
            }
            crate::lock_down_renderer();
            match std::process::Command::new("cmd.exe").args(["/C", "exit"]).status() {
                Ok(_) => std::process::exit(wcode::NOT_DENIED),
                Err(_) => std::process::exit(0),
            }
        }

        // Behaviour is the real test, but the kernel also reports what it
        // recorded - and the two can disagree if a policy word is assembled
        // wrongly. This checks the flags actually took, including the one
        // mitigation with no convenient behavioural probe (extension points,
        // which needs a third party to attempt an injection).
        "mitigation-policies-readback" => {
            crate::lock_down_renderer();
            let expect: [(_, u32, &str); 3] = [
                (ProcessDynamicCodePolicy, 1, "dynamic-code"),
                (ProcessChildProcessPolicy, 1, "child-process"),
                (ProcessExtensionPointDisablePolicy, 1, "extension-points"),
            ];
            for (policy, bit, name) in expect {
                match crate::get_mitigation_policy(policy) {
                    Ok(flags) if flags & bit == bit => {}
                    Ok(flags) => {
                        eprintln!("[selftest] {name}: expected bit {bit}, read {flags:#x}");
                        std::process::exit(wcode::WRONG_VALUE);
                    }
                    Err(e) => {
                        eprintln!("[selftest] {name}: could not read policy: {e}");
                        std::process::exit(wcode::WRONG_VALUE);
                    }
                }
            }
            std::process::exit(0);
        }

        // Integrity is mandatory access control: a low-integrity process
        // cannot write to objects labelled medium or above, which is most of
        // the user's profile. Tested behaviourally rather than by reading the
        // token back - what matters is that a write is actually refused, and
        // the checkout directory the test runs from is medium integrity.
        "low-integrity" => {
            let path = std::env::current_dir()
                .unwrap_or_else(|_| ".".into())
                .join("gosub-integrity-probe.tmp");
            if std::fs::write(&path, b"control").is_err() {
                std::process::exit(wcode::CONTROL_FAILED);
            }
            let _ = std::fs::remove_file(&path);

            crate::lock_down_renderer();

            match std::fs::write(&path, b"after") {
                Ok(()) => {
                    let _ = std::fs::remove_file(&path);
                    std::process::exit(wcode::NOT_DENIED)
                }
                Err(_) => std::process::exit(0),
            }
        }

        // The job object's memory ceiling - the `RLIMIT_AS` analogue Windows
        // otherwise lacks. Uses its own small limit rather than the engine's
        // 512 MiB so the probe stays quick; what is under test is that a job
        // memory cap binds at all, not the specific number.
        "job-memory-limit" => {
            use windows_sys::Win32::System::Memory::{
                VirtualAlloc, VirtualFree, MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_READWRITE,
            };
            const LIMIT: usize = 64 * 1024 * 1024;
            const ASK: usize = 192 * 1024 * 1024;

            // SAFETY: plain anonymous commit; freed immediately.
            let control = unsafe {
                let p = VirtualAlloc(std::ptr::null(), ASK, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE);
                if p.is_null() {
                    false
                } else {
                    VirtualFree(p, 0, MEM_RELEASE);
                    true
                }
            };
            if !control {
                std::process::exit(wcode::CONTROL_FAILED);
            }

            // SAFETY: pseudo-handle for the current process.
            let me = unsafe { windows_sys::Win32::System::Threading::GetCurrentProcess() };
            if crate::apply_job_limits(me, LIMIT).is_err() {
                std::process::exit(wcode::WRONG_VALUE);
            }

            // SAFETY: as above; the allocation is expected to be refused now.
            let after = unsafe {
                let p = VirtualAlloc(std::ptr::null(), ASK, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE);
                if p.is_null() {
                    false
                } else {
                    VirtualFree(p, 0, MEM_RELEASE);
                    true
                }
            };
            if after {
                std::process::exit(wcode::NOT_DENIED);
            }
            std::process::exit(0);
        }

        // Privileges are the ambient "may override an ACL" rights - debug
        // other processes, load drivers, take ownership. A renderer needs
        // none, and `DISABLE_MAX_PRIVILEGE` leaves exactly one
        // (SeChangeNotifyPrivilege).
        "restricted-token" => {
            // SAFETY: pseudo-handle for self; token handle out.
            let mut mine = std::ptr::null_mut();
            let opened = unsafe {
                windows_sys::Win32::System::Threading::OpenProcessToken(
                    windows_sys::Win32::System::Threading::GetCurrentProcess(),
                    windows_sys::Win32::Security::TOKEN_QUERY,
                    &mut mine,
                )
            };
            if opened == 0 {
                std::process::exit(wcode::WRONG_VALUE);
            }
            let Some(before) = privilege_count(mine) else {
                std::process::exit(wcode::WRONG_VALUE);
            };
            if before <= 1 {
                // Already privilege-free: restricting further would prove
                // nothing, so report a broken control rather than a pass.
                std::process::exit(wcode::CONTROL_FAILED);
            }

            let Some(restricted) = crate::restricted_token() else {
                std::process::exit(wcode::WRONG_VALUE);
            };
            let Some(after) = privilege_count(restricted) else {
                std::process::exit(wcode::WRONG_VALUE);
            };

            eprintln!("[selftest] privileges: {before} -> {after}");
            if after >= before {
                std::process::exit(wcode::NOT_DENIED);
            }
            std::process::exit(0);
        }

        other => {
            eprintln!("unknown Windows probe: {other}");
            std::process::exit(2);
        }
    }
}

/// Entry point for the `selftest <probe>` role.
pub fn run(probe: &str) {
    if probe == "list" {
        for name in PROBES {
            println!("{name}");
        }
        std::process::exit(0);
    }
    #[cfg(target_os = "linux")]
    run_platform_probe(probe);
    #[cfg(target_os = "macos")]
    run_macos_probe(probe);
    #[cfg(target_os = "windows")]
    run_windows_probe(probe);
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        eprintln!("no sandbox probes are compiled in for this platform: {probe}");
        std::process::exit(2);
    }
}

/// Outcome codes shared by the macOS probes. They exist because a Seatbelt
/// denial is *not* fatal: unlike seccomp's `KillProcess`, the call simply
/// returns `EPERM` and the process runs on. So the harness cannot read a signal
/// from outside - the probe has to report what it observed, and distinguishing
/// "correctly denied" from "the operation never worked anyway" is the whole
/// point.
#[cfg(target_os = "macos")]
mod code {
    /// The operation failed *before* lockdown, so the probe proves nothing:
    /// a denial afterwards would have happened regardless of the profile.
    pub const CONTROL_FAILED: i32 = 90;
    /// The operation still succeeded after lockdown - the profile is not
    /// enforcing what it claims to.
    pub const NOT_DENIED: i32 = 91;
    /// Denied, but not by the sandbox (some other errno) - reported separately
    /// so a misleading pass cannot come from an unrelated failure.
    pub const WRONG_ERROR: i32 = 92;
    /// A cap was applied but did not take the value it claims to.
    pub const WRONG_VALUE: i32 = 93;
}

/// Read back one rlimit's soft value.
#[cfg(target_os = "macos")]
fn rlimit_soft(resource: libc::c_int) -> libc::rlim_t {
    let mut rl = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: valid resource id and a valid out-pointer.
    if unsafe { libc::getrlimit(resource, &mut rl) } < 0 {
        return libc::rlim_t::MAX;
    }
    rl.rlim_cur
}

/// Can this process signal another one? Uses the target's own parent and
/// signal 0 (an existence check that still counts as a `signal` operation to
/// Seatbelt), so nothing is actually delivered.
#[cfg(target_os = "macos")]
fn try_signal_parent() -> i32 {
    // SAFETY: getppid is infallible; kill with signal 0 sends nothing.
    unsafe {
        let ppid = libc::getppid();
        if libc::kill(ppid, 0) == 0 {
            0
        } else {
            std::io::Error::last_os_error().raw_os_error().unwrap_or(-1)
        }
    }
}

/// Can this process read a sysctl? `hw.memsize` is a plain read-only value.
#[cfg(target_os = "macos")]
fn try_sysctl() -> i32 {
    let name = c"hw.memsize";
    let mut out: u64 = 0;
    let mut len = std::mem::size_of::<u64>();
    // SAFETY: NUL-terminated name, correctly sized out-buffer and length.
    unsafe {
        if libc::sysctlbyname(
            name.as_ptr(),
            std::ptr::addr_of_mut!(out).cast(),
            &mut len,
            std::ptr::null_mut(),
            0,
        ) == 0
        {
            0
        } else {
            std::io::Error::last_os_error().raw_os_error().unwrap_or(-1)
        }
    }
}

/// Fork a child that immediately exits. Returns `Ok` if the fork was permitted.
#[cfg(target_os = "macos")]
fn try_fork() -> Result<(), i32> {
    // SAFETY: the child does nothing but _exit, which is async-signal-safe.
    unsafe {
        match libc::fork() {
            -1 => Err(std::io::Error::last_os_error().raw_os_error().unwrap_or(-1)),
            0 => libc::_exit(0),
            pid => {
                libc::waitpid(pid, std::ptr::null_mut(), 0);
                Ok(())
            }
        }
    }
}

/// Can this process open a well-known readable file? `Ok` = yes.
#[cfg(target_os = "macos")]
fn try_open_file() -> std::io::Result<()> {
    std::fs::File::open("/etc/hosts").map(|_| ())
}

/// Attempt an outbound TCP connect, returning the raw errno (0 = connected).
///
/// The target is a closed port on loopback, so *unconfined* this fails fast
/// with `ECONNREFUSED` - a definite "the network stack let me try". Confined,
/// the socket or connect is refused with `EPERM` before any packet moves. The
/// two errnos are what separates "the sandbox blocked it" from "there was
/// nothing to connect to", which a bare success/failure check cannot do.
#[cfg(target_os = "macos")]
fn try_connect() -> i32 {
    // SAFETY: a plain AF_INET socket and a fully initialized sockaddr_in.
    unsafe {
        let fd = libc::socket(libc::AF_INET, libc::SOCK_STREAM, 0);
        if fd < 0 {
            return std::io::Error::last_os_error().raw_os_error().unwrap_or(-1);
        }
        let addr = libc::sockaddr_in {
            sin_len: std::mem::size_of::<libc::sockaddr_in>() as u8,
            sin_family: libc::AF_INET as libc::sa_family_t,
            sin_port: 9u16.to_be(), // discard, expected closed
            sin_addr: libc::in_addr {
                s_addr: u32::from_ne_bytes([127, 0, 0, 1]),
            },
            sin_zero: [0; 8],
        };
        let rc = libc::connect(
            fd,
            std::ptr::addr_of!(addr).cast(),
            std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t,
        );
        let errno = if rc == 0 {
            0
        } else {
            std::io::Error::last_os_error().raw_os_error().unwrap_or(-1)
        };
        libc::close(fd);
        errno
    }
}

/// Candidate Mach services the `seatbelt-mach-lookup` probe tries as its control
/// - at least one resolves in any user session's bootstrap namespace. A renderer
/// needs *none* of them; the point is to prove a lookup that works before
/// lockdown is refused after.
#[cfg(target_os = "macos")]
const MACH_CONTROL_SERVICES: &[&str] = &[
    "com.apple.system.notification_center",
    "com.apple.coreservices.launchservicesd",
    "com.apple.CoreServices.coreservicesd",
    "com.apple.system.opendirectoryd.api",
];

/// Look a Mach service up in the bootstrap namespace, returning the raw
/// `kern_return_t` (0 = a send right was handed back). `bootstrap_look_up` and
/// the `bootstrap_port` global live in libSystem but are absent from the `libc`
/// bindings, so they are declared here.
#[cfg(target_os = "macos")]
fn try_mach_lookup(name: &str) -> libc::kern_return_t {
    extern "C" {
        static mut bootstrap_port: libc::mach_port_t;
        fn bootstrap_look_up(
            bp: libc::mach_port_t,
            service_name: *const libc::c_char,
            sp: *mut libc::mach_port_t,
        ) -> libc::kern_return_t;
    }
    let cname = std::ffi::CString::new(name).expect("service name has no NUL");
    let mut port: libc::mach_port_t = 0;
    // SAFETY: valid (inherited) bootstrap port, NUL-terminated name, valid out-param.
    unsafe { bootstrap_look_up(bootstrap_port, cname.as_ptr(), &mut port) }
}

/// The macOS probe set: Seatbelt profile enforcement.
#[cfg(target_os = "macos")]
fn run_macos_probe(probe: &str) {
    match probe {
        // A renderer has no filesystem: `(deny default)` withholds `file-read*`,
        // the SBPL counterpart of `openat` being absent from the seccomp list.
        "seatbelt-file" => {
            if try_open_file().is_err() {
                std::process::exit(code::CONTROL_FAILED);
            }
            crate::lock_down_renderer();
            match try_open_file() {
                Ok(()) => std::process::exit(code::NOT_DENIED),
                Err(e) if e.raw_os_error() == Some(libc::EPERM) => std::process::exit(0),
                Err(_) => std::process::exit(code::WRONG_ERROR),
            }
        }

        // A renderer has no network. On Linux this is the empty netns plus the
        // missing socket syscalls; here it is the profile omitting
        // `network-outbound`, which is why it is worth testing separately.
        "seatbelt-network" => {
            if try_connect() != libc::ECONNREFUSED {
                std::process::exit(code::CONTROL_FAILED);
            }
            crate::lock_down_renderer();
            match try_connect() {
                0 => std::process::exit(code::NOT_DENIED),
                e if e == libc::EPERM => std::process::exit(0),
                _ => std::process::exit(code::WRONG_ERROR),
            }
        }

        // No new programs: the analogue of `execve`/`clone` being off the
        // seccomp list. `(deny default)` covers process-fork and process-exec,
        // so spawning must fail rather than replace this process.
        "seatbelt-exec" => {
            if std::process::Command::new("/usr/bin/true").status().is_err() {
                std::process::exit(code::CONTROL_FAILED);
            }
            crate::lock_down_renderer();
            match std::process::Command::new("/usr/bin/true").status() {
                Ok(_) => std::process::exit(code::NOT_DENIED),
                Err(_) => std::process::exit(0),
            }
        }

        // The positive half, and the one that keeps the others honest: the net
        // component's profile *does* grant `network-outbound`. Without this, a
        // green "renderer cannot reach the network" would be equally consistent
        // with the host having no network at all.
        "seatbelt-net-role-keeps-network" => {
            crate::lock_down_net(&[]);
            match try_connect() {
                e if e == libc::ECONNREFUSED => std::process::exit(0),
                e if e == libc::EPERM => std::process::exit(code::NOT_DENIED),
                _ => std::process::exit(code::WRONG_ERROR),
            }
        }

        // The control for every "denied" probe above: normal work must still
        // run under the profile. Without this, all the denials are equally
        // consistent with a profile so tight the component cannot function -
        // which would pass every negative test and ship a broken renderer.
        "seatbelt-baseline" => {
            crate::lock_down_renderer();
            // Allocate, compute, and write to an already-open fd: exactly what
            // a rasterizing renderer does between messages.
            let buf: Vec<u8> = (0..4096u32).map(|i| (i % 251) as u8).collect();
            let sum: u64 = buf.iter().map(|b| *b as u64).sum();
            eprintln!("[selftest] baseline computed {sum} under the profile");
            std::process::exit(0);
        }

        // `file-read*` and `file-write*` are separate SBPL operations, so a
        // read-only denial does not imply a write denial. A renderer that could
        // create files could stage a payload even without being able to read.
        "seatbelt-file-write" => {
            let path = std::env::temp_dir().join("gosub-seatbelt-probe");
            if std::fs::write(&path, b"control").is_err() {
                std::process::exit(code::CONTROL_FAILED);
            }
            let _ = std::fs::remove_file(&path);
            crate::lock_down_renderer();
            match std::fs::write(&path, b"after") {
                Ok(()) => {
                    let _ = std::fs::remove_file(&path);
                    std::process::exit(code::NOT_DENIED)
                }
                Err(e) if e.raw_os_error() == Some(libc::EPERM) => std::process::exit(0),
                Err(_) => std::process::exit(code::WRONG_ERROR),
            }
        }

        // `process-fork` is distinct from `process-exec`: a renderer that can
        // fork can multiply itself even with exec denied.
        "seatbelt-fork" => {
            if try_fork().is_err() {
                std::process::exit(code::CONTROL_FAILED);
            }
            crate::lock_down_renderer();
            match try_fork() {
                Ok(()) => std::process::exit(code::NOT_DENIED),
                Err(_) => std::process::exit(0),
            }
        }

        // Tests the *precision* of the profile, not just its existence. The
        // grant is `(allow signal (target self))` - scoped deliberately - so
        // signalling any other process must still be refused. If `(target
        // self)` were dropped or widened, every other probe here would still
        // pass and only this one would notice.
        "seatbelt-signal-other" => {
            if try_signal_parent() != 0 {
                std::process::exit(code::CONTROL_FAILED);
            }
            crate::lock_down_renderer();
            match try_signal_parent() {
                0 => std::process::exit(code::NOT_DENIED),
                e if e == libc::EPERM => std::process::exit(0),
                _ => std::process::exit(code::WRONG_ERROR),
            }
        }

        // The module docs claim the profile grants no `sysctl-read`. Nothing
        // verified that claim; sysctls leak host details (memory size, CPU
        // count, boot time) useful for fingerprinting and exploit tuning.
        "seatbelt-sysctl" => {
            if try_sysctl() != 0 {
                std::process::exit(code::CONTROL_FAILED);
            }
            crate::lock_down_renderer();
            match try_sysctl() {
                0 => std::process::exit(code::NOT_DENIED),
                e if e == libc::EPERM => std::process::exit(0),
                _ => std::process::exit(code::WRONG_ERROR),
            }
        }

        // `mach-lookup` (the bootstrap namespace: WindowServer, launchd
        // services) is the classic macOS sandbox-escape surface; `(deny
        // default)` covers it. Control: a well-known service resolves before
        // lockdown, then the same lookup must be refused after. A Seatbelt
        // mach-lookup denial surfaces as a bootstrap error - observed
        // BOOTSTRAP_NOT_PRIVILEGED (1100) on an M1, not EPERM - so this checks
        // for any failure rather than pinning one errno.
        "seatbelt-mach-lookup" => {
            let control = MACH_CONTROL_SERVICES.iter().copied().find(|n| try_mach_lookup(n) == 0);
            let Some(name) = control else {
                eprintln!("[selftest] mach-lookup: no control service resolved pre-lockdown — probe proves nothing");
                std::process::exit(code::CONTROL_FAILED);
            };
            eprintln!("[selftest] mach-lookup: control service {name} resolved before lockdown");
            crate::lock_down_renderer();
            let kr = try_mach_lookup(name);
            if kr == 0 {
                eprintln!("[selftest] mach-lookup: {name} still resolved after lockdown — NOT denied");
                std::process::exit(code::NOT_DENIED);
            }
            eprintln!("[selftest] mach-lookup: denied after lockdown (kern_return {kr})");
            std::process::exit(0);
        }

        // The rlimits are a mechanism entirely separate from Seatbelt and were
        // wholly unverified on macOS. Checks the two caps that actually change
        // a value here - note `RLIMIT_CORE` is deliberately not asserted,
        // because macOS already defaults it to 0 and proving a no-op proves
        // nothing. `RLIMIT_AS` is absent by design (see the backend docs).
        "rlimits" => {
            let nofile_before = rlimit_soft(libc::RLIMIT_NOFILE);
            // SAFETY: PRIO_PROCESS with pid 0 targets this process.
            let prio_before = unsafe { libc::getpriority(libc::PRIO_PROCESS, 0) };
            if nofile_before == 128 || prio_before == 10 {
                // Already at the target value: the check below would pass
                // without the call having done anything.
                std::process::exit(code::CONTROL_FAILED);
            }
            if crate::apply_child_rlimits().is_err() {
                std::process::exit(code::WRONG_ERROR);
            }
            let prio_after = unsafe { libc::getpriority(libc::PRIO_PROCESS, 0) };
            if rlimit_soft(libc::RLIMIT_NOFILE) != 128 || prio_after != 10 {
                std::process::exit(code::WRONG_VALUE);
            }
            std::process::exit(0);
        }

        // The inbound direction, and the one mechanism here that is not
        // Seatbelt. This verifies only that the kernel *accepts*
        // `PT_DENY_ATTACH` - deliberately a weaker claim than the Linux
        // `no-ptrace` probe makes, and named so it cannot be misread as more.
        "ptrace-deny-accepted" => {
            // SAFETY: PT_DENY_ATTACH takes no addr/data and affects only us.
            let rc = unsafe { libc::ptrace(libc::PT_DENY_ATTACH, 0, std::ptr::null_mut(), 0) };
            if rc < 0 {
                std::process::exit(code::NOT_DENIED);
            }
            std::process::exit(0);
        }

        // A filesystem service is path-scoped to *its own* directory - the SBPL
        // counterpart of the Linux services' Landlock ruleset. It may read+write
        // inside its declared path but is denied outside, even though the profile
        // is a filesystem-service profile. Mirrors the Linux `service-landlock`
        // probe. The control (both work pre-lockdown) keeps a broken path or a
        // read-only temp dir from passing this vacuously.
        "seatbelt-service-scope" => {
            let dir = std::env::temp_dir().join("gosub-seatbelt-scope");
            let _ = std::fs::create_dir_all(&dir);
            let inside = dir.join("inside.tmp");
            let outside = std::env::temp_dir().join("gosub-seatbelt-outside.tmp");
            let _ = std::fs::write(&outside, b"pre");
            if std::fs::write(&inside, b"x").is_err() || std::fs::read(&outside).is_err() {
                std::process::exit(code::CONTROL_FAILED);
            }

            crate::lock_down_service(
                "probe",
                crate::ServiceCaps {
                    filesystem: true,
                    device: false,
                },
                &[(dir.as_path(), true)],
            );

            // Inside the scope: writing must still work (the allow rule took).
            if let Err(e) = std::fs::write(&inside, b"after") {
                eprintln!(
                    "[selftest] seatbelt-service-scope: inside write DENIED errno={:?} path={}",
                    e.raw_os_error(),
                    inside.display()
                );
                std::process::exit(code::WRONG_VALUE);
            }
            // Outside the scope: reading must be refused with EPERM.
            match std::fs::read(&outside) {
                Ok(_) => std::process::exit(code::NOT_DENIED),
                Err(e) if e.raw_os_error() == Some(libc::EPERM) => std::process::exit(0),
                Err(_) => std::process::exit(code::WRONG_ERROR),
            }
        }

        other => {
            eprintln!("unknown macOS probe: {other}");
            std::process::exit(2);
        }
    }
}

/// The Linux probe set. Each either exits 0 (property holds) or dies on
/// `SIGSYS` (the sandbox killed a forbidden operation), which the harness
/// observes from outside.
///
/// ## Shape note for other platforms
#[cfg(target_os = "linux")]
fn run_platform_probe(probe: &str) {
    // The netns probe must run *before* the seccomp lockdown: verifying the
    // namespace is empty means enumerating interfaces, and a locked-down
    // renderer has no `openat`/`socket` with which to look. It asserts the
    // layer underneath the filter, so it is checked underneath it too.
    if probe == "netns" {
        // Read the interface list from procfs, NOT `/sys/class/net`: sysfs
        // reports the namespace it was *mounted* in, so it keeps showing the
        // host's interfaces after an unshare and would make this probe pass
        // vacuously. `/proc/self/net` follows the calling task's netns.
        let interfaces = || -> Vec<String> {
            let dev = std::fs::read_to_string("/proc/self/net/dev").expect("read /proc/self/net/dev");
            let mut names: Vec<String> = dev
                .lines()
                .skip(2) // two header lines
                .filter_map(|l| l.split(':').next())
                .map(|n| n.trim().to_string())
                .collect();
            names.sort();
            names
        };

        let ns_link = |kind: &str| std::fs::read_link(format!("/proc/self/ns/{kind}")).expect("read ns link");
        // `isolate_network` unshares net + ipc + uts together, so all three
        // namespace ids must change - checking only net would miss a regression
        // that dropped ipc/uts from the flag set.
        let (net0, ipc0, uts0) = (ns_link("net"), ns_link("ipc"), ns_link("uts"));
        assert!(
            interfaces().len() > 1,
            "host netns looks empty already — probe proves nothing"
        );

        crate::isolate_namespaces(crate::NamespaceIsolation::Full).expect("unshare namespaces");

        // The network namespace must actually have changed, and the new one must
        // hold nothing but loopback: no route off this machine exists at all.
        assert_ne!(net0, ns_link("net"), "still in the host network namespace");
        assert_eq!(interfaces(), vec!["lo".to_string()], "netns is not empty");
        // ...and the IPC and UTS namespaces changed too (defense in depth).
        assert_ne!(ipc0, ns_link("ipc"), "still in the host IPC namespace");
        assert_ne!(uts0, ns_link("uts"), "still in the host UTS namespace");
        std::process::exit(0);
    }

    if probe == "pidns" {
        // `isolate_network` also requests a PID namespace, which (because
        // `unshare(CLONE_NEWPID)` is lazy) places the caller's *children* - not
        // the caller - in it. So prove it the way the fork server relies on:
        // after the unshare, a forked child must be PID 1 of a fresh namespace.
        // Best-effort like the real path - where the kernel refused CLONE_NEWPID
        // and it fell back, the child is not PID 1, and we skip (exit 0) rather
        // than fail a host that cannot make PID namespaces.
        crate::isolate_namespaces(crate::NamespaceIsolation::Full).expect("unshare namespaces");
        // SAFETY: single-threaded probe; child runs only getpid + _exit.
        match unsafe { libc::fork() } {
            -1 => {
                eprintln!("[selftest] pidns: fork failed: {}", std::io::Error::last_os_error());
                std::process::exit(1);
            }
            0 => {
                // Raw getpid (not glibc's possibly-cached wrapper): 1 ⇒ we are
                // init of a fresh PID namespace. Carry the answer in the code.
                let pid = unsafe { libc::syscall(libc::SYS_getpid) };
                unsafe { libc::_exit(if pid == 1 { 0 } else { 40 }) };
            }
            child => {
                let mut st = 0;
                if unsafe { libc::waitpid(child, &mut st, 0) } != child {
                    eprintln!("[selftest] pidns: could not reap the child");
                    std::process::exit(1);
                }
                match libc::WEXITSTATUS(st) {
                    0 => {
                        eprintln!("[selftest] pidns: child is PID 1 of a fresh namespace (ok)");
                        std::process::exit(0);
                    }
                    40 => {
                        eprintln!("[selftest] pidns: CLONE_NEWPID unavailable (fell back) — skipping");
                        std::process::exit(0);
                    }
                    other => {
                        eprintln!("[selftest] pidns: unexpected child exit {other}");
                        std::process::exit(1);
                    }
                }
            }
        }
    }

    // Also pre-lockdown, for the same reason: `prctl` is not on the allowlist,
    // so `PR_GET_DUMPABLE` after lockdown would itself be a fatal SIGSYS.
    if probe == "no-ptrace" {
        // Prove the property itself - that a `PTRACE_ATTACH` is refused -
        // rather than just reading back the flag we set. The attach has to run
        // parent→child: Yama's default `ptrace_scope=1` permits tracing only
        // descendants, so a child attaching to its parent would fail for
        // reasons that have nothing to do with the dumpable flag and the test
        // would pass vacuously.
        assert!(
            attach_refused(false).is_none(),
            "control: an unprotected child should be traceable"
        );
        let errno = attach_refused(true).expect("a non-dumpable child must refuse PTRACE_ATTACH");
        assert_eq!(errno, libc::EPERM, "expected EPERM, got errno {errno}");
        std::process::exit(0);
    }

    // The broker's *loose* Landlock: read/exec anywhere, write only beneath the
    // temp dir. Runs before any seccomp lockdown (it needs `openat` to test file
    // writes). Prove both halves - a write inside temp succeeds, a write outside
    // is denied (`EACCES`) - with a control that shows the outside write worked
    // *before* lockdown, so the denial is Landlock's and not a plain permission
    // error. Skips cleanly where Landlock is unavailable, like the service probe.
    if probe == "broker-landlock" {
        if !crate::landlock_available() {
            eprintln!("[selftest] landlock unavailable — skipping");
            std::process::exit(0);
        }
        let inside = std::env::temp_dir().join("gosub-broker-inside.tmp");
        // Outside temp: the cwd the probe was spawned from (the crate root),
        // which the user can normally write - a real control, not `/`.
        let outside = std::env::current_dir()
            .unwrap_or_else(|_| "/".into())
            .join("gosub-broker-outside.tmp");

        let control_ok = std::fs::write(&outside, b"pre").is_ok();
        let _ = std::fs::remove_file(&outside);

        crate::lock_down_broker();

        let inside_ok = std::fs::write(&inside, b"x").is_ok();
        let _ = std::fs::remove_file(&inside);
        let outside_denied = match std::fs::write(&outside, b"x") {
            Err(e) if e.raw_os_error() == Some(libc::EACCES) => true,
            Ok(()) => {
                let _ = std::fs::remove_file(&outside); // Landlock didn't bind — clean up
                false
            }
            Err(_) => false,
        };
        eprintln!(
            "[selftest] broker-landlock: control_ok={control_ok} inside_ok={inside_ok} \
             outside_denied={outside_denied}"
        );
        std::process::exit(if control_ok && inside_ok && outside_denied {
            0
        } else {
            1
        });
    }

    // The broker's deny-list must bind: `ptrace` is on it, so any call is a
    // fatal SIGSYS. `PTRACE_TRACEME` needs no target and would ordinarily
    // succeed, so surviving the call means the deny-list did not bind. The
    // positive case is the multi-process demo itself (spawns, execs, opens).
    if probe == "broker-seccomp" {
        crate::lock_down_broker();
        // SAFETY: a plain ptrace request; the point is that the syscall is trapped
        // before it returns, not what it would have done.
        unsafe { libc::ptrace(libc::PTRACE_TRACEME, 0, 0, 0) };
        eprintln!("[selftest] broker-seccomp: ptrace was NOT denied — deny-list did not bind");
        std::process::exit(1);
    }

    // The deny-list must close the *fd-based* mount API too, not only the classic
    // `mount`/`pivot_root` - otherwise a compromised broker could `fsopen`+
    // `fsmount`+`move_mount` its way to the same escape. `fsopen` is on the list,
    // so a raw call to it is a fatal `SIGSYS`; reaching the line past it means the
    // new mount API was left open, and we exit non-zero to say so.
    if probe == "broker-seccomp-mount" {
        crate::lock_down_broker();
        // SAFETY: a raw `fsopen` with a dummy fs name; the point is that the
        // syscall traps before it returns, not what it would have opened.
        let name = b"tmpfs\0";
        unsafe { libc::syscall(libc::SYS_fsopen, name.as_ptr(), 0u32) };
        eprintln!(
            "[selftest] broker-seccomp-mount: fsopen was NOT denied — the fd-based mount API is not on the deny-list"
        );
        std::process::exit(1);
    }

    // cgroup v2 per-child memory bound: place this process in a child cgroup with
    // a known `memory.max` and read it back, proving the limit actually binds.
    // Best-effort like the Landlock probes - where cgroup v2 memory delegation is
    // unavailable (a shared login/tmux scope, no `Delegate=yes`), it skips (exit
    // 0) rather than failing an untestable host. Run under a delegated scope
    // (`systemd-run --user -p Delegate=yes --scope …`) to exercise the real path.
    // The self-capturing crash reporter must fire *and* still let the process
    // die with the original signal. Under the renderer lockdown (so this also
    // proves the handler works within the seccomp filter - it uses only `write`
    // + `sigaction`, and re-dies by restoring `SIG_DFL` and returning rather than
    // a filter-blocked `tgkill`), dereference a null pointer to raise SIGSEGV.
    // The handler writes a `[crash]` line, restores the default, and returns; the
    // faulting instruction re-executes and the process dies with SIGSEGV, which
    // the parent observes. Reaching the line below would mean the fault was
    // swallowed - a bug.
    if probe == "crash-report" {
        crate::lock_down_renderer();
        // SAFETY: an intentional wild read of address 0 to trigger SIGSEGV;
        // `read_volatile` is not optimized away.
        let _ = unsafe { std::ptr::read_volatile(std::ptr::null::<u8>()) };
        eprintln!("[selftest] crash-report: SIGSEGV did not terminate the process");
        std::process::exit(1);
    }

    if probe == "cgroup-memory-limit" {
        const WANT: u64 = 64 * 1024 * 1024;
        match crate::cgroup_confine_self(WANT) {
            None => {
                eprintln!("[selftest] cgroup v2 memory delegation unavailable — skipping");
                std::process::exit(0);
            }
            Some(Ok(got)) if got == WANT => {
                eprintln!("[selftest] cgroup-memory-limit: memory.max read back {got} (ok)");
                std::process::exit(0);
            }
            Some(Ok(got)) => {
                eprintln!("[selftest] cgroup-memory-limit: memory.max was {got}, wanted {WANT}");
                std::process::exit(1);
            }
            Some(Err(e)) => {
                eprintln!("[selftest] cgroup-memory-limit: could not apply the limit ({e})");
                std::process::exit(1);
            }
        }
    }

    // Fork-server probes: this role's filter is not the renderer's, and it is
    // inherited by every renderer forked under it - so it gets its own
    // lockdown here rather than falling through to `lock_down_renderer`.
    // Service-filter probes: each confines itself with a service filter, then
    // tests one syscall. They run before the renderer lockdown below because
    // they need a *different* filter (a superset of the baseline).
    if let Some(op) = probe.strip_prefix("service-") {
        use crate::ServiceCaps;
        match op {
            // The filesystem filter must permit `openat` - the whole reason a
            // storage/font service is a separate process. Allowed = the syscall
            // returns (fd or errno) rather than a fatal SIGSYS; clean exit.
            "fs-openat" => {
                crate::lock_down_service(
                    "probe",
                    ServiceCaps {
                        filesystem: true,
                        device: false,
                    },
                    &[],
                );
                // SAFETY: a NUL-terminated path and standard open flags.
                let fd = unsafe { libc::openat(libc::AT_FDCWD, c"/dev/null".as_ptr(), libc::O_RDONLY) };
                if fd >= 0 {
                    unsafe { libc::close(fd) };
                }
                std::process::exit(0);
            }
            // ...but it is a *superset of the baseline*, not a blank cheque:
            // network is still denied. Reached (clean exit) only if the filter
            // wrongly allowed `socket`; otherwise SIGSYS.
            "fs-no-socket" => {
                crate::lock_down_service(
                    "probe",
                    ServiceCaps {
                        filesystem: true,
                        device: false,
                    },
                    &[],
                );
                // SAFETY: obtaining a socket; expected to be fatal.
                let _ = unsafe { libc::socket(libc::AF_INET, libc::SOCK_STREAM, 0) };
                std::process::exit(0);
            }
            // The device filter must permit `ioctl` (how a real audio/GPU
            // service drives its device). An unsupported request returns ENOTTY
            // rather than being killed; clean exit = the syscall was allowed.
            "device-ioctl" => {
                crate::lock_down_service(
                    "probe",
                    ServiceCaps {
                        filesystem: false,
                        device: true,
                    },
                    &[],
                );
                let mut winsz: libc::winsize = unsafe { std::mem::zeroed() };
                // SAFETY: TIOCGWINSZ with a valid out-struct; fd 2 may not be a
                // tty, in which case it errors - which is fine, we only need the
                // syscall to be permitted rather than killed.
                let _ = unsafe { libc::ioctl(2, libc::TIOCGWINSZ, &mut winsz) };
                std::process::exit(0);
            }
            // Landlock: the path-level confinement seccomp cannot do. Scoped to
            // a temp dir, the service may open files *inside* it but is denied
            // (EACCES) *outside* - even though seccomp still permits `openat`.
            // Skips cleanly where Landlock is unavailable (kernel/config), like
            // the macOS ptrace probe, so it never fails for an untestable host.
            "landlock" => {
                if !crate::landlock_available() {
                    eprintln!("[selftest] landlock unavailable on this kernel — skipping");
                    std::process::exit(0);
                }
                let dir = std::env::temp_dir().join("gosub-ll-probe");
                let _ = std::fs::create_dir_all(&dir);
                let inside = dir.join("inside.tmp");
                // A file *outside* the ruleset, created before lockdown so it
                // certainly exists (an EACCES below is Landlock, not ENOENT).
                let outside = std::env::temp_dir().join("gosub-ll-outside.tmp");
                let _ = std::fs::write(&outside, b"pre");

                crate::lock_down_service(
                    "probe",
                    ServiceCaps {
                        filesystem: true,
                        device: false,
                    },
                    &[(dir.as_path(), true)],
                );

                // Inside the scope: creating/writing must work.
                let ok_inside = std::fs::write(&inside, b"x").is_ok();
                // Outside the scope: reading must be refused with EACCES.
                let denied_outside = matches!(
                    std::fs::read(&outside),
                    Err(e) if e.raw_os_error() == Some(libc::EACCES)
                );
                eprintln!("[selftest] landlock: inside_ok={ok_inside} outside_denied={denied_outside}");
                std::process::exit(if ok_inside && denied_outside { 0 } else { 1 });
            }
            other => {
                eprintln!("unknown service probe: {other}");
                std::process::exit(2);
            }
        }
    }

    if let Some(op) = probe.strip_prefix("forkserver-") {
        crate::lock_down_fork_server();
        match op {
            // The positive case, and the one that actually bites: this filter
            // is inherited across `fork`, so anything it forgets kills the
            // renderer instead of the fork server. Forking, reaping, and the
            // `F_DUPFD_CLOEXEC` a child needs to split its endpoint before its
            // own lockdown must all still work. Clean exit = pass.
            "can-fork" => unsafe {
                let pid = libc::fork();
                assert!(pid >= 0, "fork refused under the fork-server filter");
                if pid == 0 {
                    libc::_exit(0);
                }
                let mut status = 0;
                assert_eq!(libc::waitpid(pid, &mut status, 0), pid, "cannot reap");
                // What `Endpoint::from_channel` does in a freshly forked child.
                let dup = libc::fcntl(2, libc::F_DUPFD_CLOEXEC, 0);
                assert!(dup >= 0, "endpoint split would fail in a forked child");
                std::process::exit(0);
            },

            // The canary must *detect*, not merely pass. Runs it against a
            // filter with one syscall deliberately removed; the canary is
            // expected to abort the process, so a clean exit here is a failure.
            "canary-gap" => crate::canary_must_detect_a_missing_syscall(),

            // The fork server forks; it never execs. Reached only if the
            // allowlist let `execve` through.
            "no-exec" => unsafe {
                let path = b"/bin/true\0";
                let argv = [path.as_ptr().cast::<libc::c_char>(), std::ptr::null()];
                let _ = libc::execve(
                    path.as_ptr().cast::<libc::c_char>(),
                    argv.as_ptr(),
                    [std::ptr::null()].as_ptr(),
                );
                std::process::exit(0);
            },

            // It talks only on the control fd it was handed. A new socket must
            // be fatal, exactly as for a renderer.
            "no-socket" => unsafe {
                let _ = libc::socket(libc::AF_INET, libc::SOCK_STREAM, 0);
                std::process::exit(0);
            },

            // A plain fork is allowed (see `can-fork`), but a `clone` that
            // unshares a namespace is not: with `clone3` ENOSYS'd, the
            // register-visible `clone` flags are masked against
            // CLONE_NEW*/CLONE_THREAD/CLONE_VM. A CLONE_NEWUSER clone must trap
            // with SIGSYS before the kernel checks userns permissions; a clean
            // exit means the mask let a dangerous flag through.
            "no-newuser-clone" => unsafe {
                // Raw `clone` with SIGCHLD (fork semantics) so a NULL stack is a
                // copy-on-write fork; `flags` is arg 0 on both x86_64 and
                // aarch64. If the filter works this never returns.
                let flags = (libc::CLONE_NEWUSER | libc::SIGCHLD) as libc::c_long;
                let ret = libc::syscall(libc::SYS_clone, flags, 0, 0, 0, 0);
                if ret == 0 {
                    libc::_exit(0); // the child, if the clone wrongly succeeded
                }
                std::process::exit(0); // parent reached here → not denied
            },

            other => {
                eprintln!("unknown fork-server probe: {other}");
                std::process::exit(2);
            }
        }
    }

    // Drop to the renderer's privileges, exactly as a real renderer does.
    crate::lock_down_renderer();

    match probe {
        // The sandbox must NOT kill an allowed program: only reads/writes on
        // existing fds, memory, and exit. Clean exit = pass.
        "baseline" => std::process::exit(0),

        // W^X: turning writable memory executable must be fatal. We reach the
        // exit only if the argument filter FAILED to trap the PROT_EXEC.
        "mprotect-exec" => unsafe {
            let p = libc::mmap(
                std::ptr::null_mut(),
                4096,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
                -1,
                0,
            );
            let _ = libc::mprotect(p, 4096, libc::PROT_READ | libc::PROT_EXEC);
            std::process::exit(0);
        },

        // Network: a renderer has no socket family, so obtaining a socket must
        // be fatal. Reached only if the allowlist let it through.
        "socket" => unsafe {
            let _ = libc::socket(libc::AF_INET, libc::SOCK_STREAM, 0);
            std::process::exit(0);
        },

        // The full shared-memory tile producer dance (memfd_create, ftruncate,
        // mmap write, munmap, fcntl F_ADD_SEALS) must survive the sandbox -
        // it's how a confined renderer ships every frame. Clean exit = pass.
        "memfd-seal" => {
            gosub_ipc::shm::create_sealed_tile(64, 64, |buf| buf.fill(0xCD)).expect("sealed tile under sandbox");
            std::process::exit(0);
        }

        // fcntl is allowed *only* for the seal commands; any other command -
        // here F_DUPFD, an fd-fabrication primitive - must be fatal. Reached
        // only if the argument filter failed.
        "fcntl-dupfd" => unsafe {
            let _ = libc::fcntl(2, libc::F_DUPFD, 0);
            std::process::exit(0);
        },

        // The full ring-buffer dance (memfd + size seals, RW mapping, cursor
        // atomics, drain) must survive the sandbox - it's how a confined
        // renderer receives every large fetch body. Single-threaded (the
        // sandbox has no clone), so the body must fit the window: write it
        // all, finish, then consume. Clean exit = pass.
        "ring" => {
            let (mut producer, fd) =
                gosub_ipc::ring::RingProducer::create(64 * 1024).expect("ring create under sandbox");
            let body: Vec<u8> = (0..16 * 1024).map(|i| (i % 251) as u8).collect();
            producer.write_all(&body).expect("ring write under sandbox");
            producer.finish();
            let got = gosub_ipc::ring::consume(fd, body.len() as u64).expect("ring consume under sandbox");
            assert_eq!(got, body, "ring bytes corrupted under sandbox");
            std::process::exit(0);
        }

        other => {
            eprintln!("unknown selftest probe: {other}");
            std::process::exit(2);
        }
    }
}
