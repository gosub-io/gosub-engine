//! OS-level privilege capping for the engine and its child components.
//!
//! ## The seam
//!
//! ### The contract assumes self-application

// How a child process is created. Owned rather than delegated to
// `std::process::Command` because Windows access controls must be supplied at
// `CreateProcess` time — see the module docs.
#[cfg(feature = "multi-process")]
pub mod spawn;
// Compiled on every platform (not just those with a backend) so a test suite can
// query the probe inventory anywhere: a platform with no probes must fail loudly
// rather than silently skip its enforcement tests.
#[cfg(feature = "multi-process")]
pub mod selftest;

// --- platform seam: the only place a sandbox `target_os` cfg lives ---
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
use linux as imp;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
use macos as imp;

#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
use windows as imp;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod unsupported;
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
use unsupported as imp;

// --- public API: thin, cfg-free wrappers over the selected backend ---

/// Mark the calling process non-dumpable, closing the *inbound* debugging
/// surface so another same-user process cannot attach a debugger and read our
/// address space (for the engine: the cookie jar in cleartext). Best-effort
/// hardening — warns rather than aborts on failure. Must be called *after*
/// `exec` (the flag does not survive it) but is inherited across `fork`.
pub fn deny_debugger_attach() {
    imp::deny_debugger_attach();
}

/// Impose resource ceilings (committed heap plus an address-space ceiling, fd
/// count, no core dumps, lowered scheduling priority) on a child. Called from
/// `pre_exec`, so it must stay async-signal-safe. rlimits only ever lower, so a
/// child cannot undo them.
#[cfg(feature = "multi-process")]
pub fn apply_child_rlimits() -> std::io::Result<()> {
    imp::apply_child_rlimits()
}

/// Isolate a child's namespaces when `enable` is set (content processes and
/// services), leaving them in place otherwise (the net component). On Linux this
/// unshares the network namespace (the load-bearing one) plus IPC and UTS as
/// defense in depth, and (best-effort) a PID namespace — which, because the
/// unshare is lazy, isolates the *fork server's forked renderers* rather than the
/// caller (see the backend docs). The mount namespace is deliberately left out
/// for concrete reasons (see the backend docs). Called from `pre_exec`, so it
/// must stay async-signal-safe. On platforms without namespaces this is deferred
/// into the lockdown profile — see the backend docs.
#[cfg(feature = "multi-process")]
pub fn isolate_network(enable: bool) -> std::io::Result<()> {
    imp::isolate_network(enable)
}

/// Confine a renderer to pixels only: no network, no files, no new programs.
/// Called once the IPC link is connected. Fail-closed — the backend aborts the
/// process rather than let a renderer meant to be confined run unconfined.
#[cfg(feature = "multi-process")]
pub fn lock_down_renderer() {
    imp::lock_down_renderer();
}

/// Confine the net component: the renderer's restrictions minus the network,
/// which is the one privilege this role keeps. Called once the IPC link is
/// connected. Fail-closed.
#[cfg(feature = "multi-process")]
pub fn lock_down_net(fs_allow: &[(&std::path::Path, bool)]) {
    imp::lock_down_net(fs_allow);
}

/// The read-only paths [`lock_down_net`] normally wants: resolver configuration
/// and the system trust store, filtered to those that exist on this host.
/// Empty off Linux, where confinement gates files another way.
#[cfg(feature = "multi-process")]
pub fn net_filesystem_paths() -> Vec<std::path::PathBuf> {
    #[cfg(target_os = "linux")]
    {
        imp::net_filesystem_paths()
    }
    #[cfg(not(target_os = "linux"))]
    {
        Vec::new()
    }
}

/// Confine the **vault** (cookie store): the tightest filter of any role — the
/// bare content baseline (no network, files, devices, or exec) plus
/// non-dumpable. It holds secrets, so it gets the least authority of any
/// process. Linux only (the vault is Linux-only). Fail-closed.
#[cfg(all(feature = "multi-process", target_os = "linux"))]
pub fn lock_down_vault() {
    imp::lock_down_vault();
}

/// What extra capability an engine-spawned service needs beyond the content
/// baseline. Unlike a renderer or the decoder, these roles need a privilege the
/// zygote gave up (filesystem or device access), which is why each is spawned
/// from the engine with its own filter rather than forked from the fork server.
#[derive(Clone, Copy)]
pub struct ServiceCaps {
    /// Needs to open files (font, storage). Adds `openat` on Linux.
    pub filesystem: bool,
    /// Needs a device node + `ioctl` (audio, GPU). Adds `openat` + `ioctl`.
    pub device: bool,
}

/// Confine an engine-spawned service to the content baseline plus exactly the
/// capability `caps` selects. `name` is the label in its lockdown banner.
#[cfg(feature = "multi-process")]
pub fn lock_down_service(name: &str, caps: ServiceCaps, fs_allow: &[(&std::path::Path, bool)]) {
    imp::lock_down_service(name, caps.filesystem, caps.device, fs_allow);
}

/// Apply parent-side confinement to a child that has just been spawned.
// The explicit `return`s keep each platform arm self-contained; without them
// the arms would have to be mutually exclusive expressions, which the trailing
// fallback block cannot be.
#[allow(clippy::needless_return)]
#[cfg(feature = "multi-process")]
pub fn confine_spawned_child(child: &crate::spawn::Child) -> std::io::Result<()> {
    #[cfg(target_os = "windows")]
    {
        return imp::confine_spawned_child(child.raw_handle());
    }
    #[cfg(target_os = "linux")]
    {
        // Best-effort cgroup memory bound (never fatal); see the backend.
        return imp::confine_spawned_child(child.id());
    }
    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    {
        let _ = child;
        Ok(())
    }
}

/// Tear down the broker's cgroup subtree at shutdown, symmetric to
/// [`confine_spawned_child`]. Best-effort and safe to call unconditionally: a
/// no-op where no subtree was set up, and outside Linux. Call once every child
/// has been reaped, so the per-child cgroups are empty and removable.
#[cfg(feature = "multi-process")]
pub fn cleanup_spawned_cgroups() {
    #[cfg(target_os = "linux")]
    imp::cleanup_spawned_cgroups();
}

/// Arm an alternate signal stack for the **current** thread, so the self-capturing
/// crash reporter can still run when this thread's own stack overflows. The
/// broker is multithreaded and the reporter's altstack is per-thread, so each
/// engine thread calls this at startup. A no-op off Linux (only the Linux backend
/// installs the crash reporter) and safe to call unconditionally.
pub fn install_thread_crash_altstack() {
    #[cfg(all(feature = "multi-process", target_os = "linux"))]
    imp::install_thread_crash_altstack();
}

/// Test hook for the `cgroup-memory-limit` probe: bound this process's memory via
/// cgroup v2 `memory.max` and read the ceiling back, or `None` where cgroup v2
/// memory delegation is unavailable (the probe then skips). Linux only.
#[cfg(all(feature = "multi-process", target_os = "linux"))]
pub fn cgroup_confine_self(limit: u64) -> Option<std::io::Result<u64>> {
    imp::cgroup_confine_self(limit)
}

/// Build a restricted primary token for a Windows child, or `None` if the host
/// refuses (the spawner then falls back to the inherited token). Windows only.
#[cfg(all(feature = "multi-process", target_os = "windows"))]
pub fn restricted_token() -> Option<::windows_sys::Win32::Foundation::HANDLE> {
    imp::restricted_token()
}

/// The AppContainer (lowbox) identity for a Windows child — the capability
/// sandbox that gives content roles no network and the net component
/// `internetClient`. Windows only. See the backend for the image-loading caveat.
#[cfg(all(feature = "multi-process", target_os = "windows"))]
pub use imp::AppContainerIdentity;

/// Build the AppContainer identity for a child (`internet` grants the
/// `internetClient` capability), or `None` if the SIDs cannot be built (the
/// spawner then falls back to the restricted-token path). Windows only.
#[cfg(all(feature = "multi-process", target_os = "windows"))]
pub fn app_container_identity(name: &str, internet: bool) -> Option<AppContainerIdentity> {
    imp::app_container_identity(name, internet)
}

/// Grant ALL APPLICATION PACKAGES read+execute on `path` so an AppContainer
/// child can load the image (the install-time ACL, done at spawn). Windows only.
#[cfg(all(feature = "multi-process", target_os = "windows"))]
pub fn grant_app_package_execute(path: &std::path::Path) -> std::io::Result<()> {
    imp::grant_app_package_execute(path)
}

/// Give a service's own AppContainer access to its file/directory (`writable`
/// also relabels it Low integrity) — the Windows analogue of the Linux services'
/// `openat` + Landlock to their own path. Windows only.
#[cfg(all(feature = "multi-process", target_os = "windows"))]
pub fn grant_container_path_access(
    path: &std::path::Path,
    container_sid: *mut std::ffi::c_void,
    writable: bool,
) -> std::io::Result<()> {
    imp::grant_container_path_access(path, container_sid, writable)
}

/// Apply a job-object memory cap to a process. Exposed for the probe suite,
/// which assigns the caps to itself to verify they bind. Windows only.
#[cfg(all(feature = "multi-process", target_os = "windows"))]
pub fn apply_job_limits(process: ::windows_sys::Win32::Foundation::HANDLE, memory_limit: usize) -> std::io::Result<()> {
    imp::apply_job_limits(process, memory_limit)
}

/// Read back a Windows process mitigation policy's flag word, so a probe can
/// confirm the kernel recorded what the backend asked for. Windows only.
#[cfg(all(feature = "multi-process", target_os = "windows"))]
pub fn get_mitigation_policy(
    policy: ::windows_sys::Win32::System::Threading::PROCESS_MITIGATION_POLICY,
) -> std::io::Result<u32> {
    imp::get_policy(policy)
}

/// Whether Landlock (path-level filesystem confinement) is usable on this
/// kernel. Linux only; used by the probe to skip cleanly where it is absent.
#[cfg(all(feature = "multi-process", target_os = "linux"))]
pub fn landlock_available() -> bool {
    imp::landlock_available()
}

/// Confine the **broker** (engine) process: a *loose* sandbox — like a browser's
/// main process — for the one process that holds every secret and deserializes
/// untrusted frames. It cannot be tightened to a renderer's degree (it must spawn
/// children, exec their libraries, thread, and open files and sockets), but two
/// blast radii can be reduced: a **Landlock** ruleset limits *writes* to the temp
/// dir (read/exec stay open), and a **deny-list seccomp filter** removes the
/// escalation syscalls it never uses (`ptrace`, kernel-module/`kexec`/`bpf`, the
/// keyring, `mount`/`setns`, …) while allowing everything else. Called by the
/// binary on its main thread before the engine starts, so every engine thread and
/// child inherits both. Linux only; a no-op elsewhere (a macOS Seatbelt broker
/// profile would be the equivalent, and is not built yet). Best-effort: a kernel
/// missing either mechanism leaves that layer off rather than aborting.
#[cfg(all(feature = "multi-process", target_os = "linux"))]
pub fn lock_down_broker() {
    imp::lock_down_broker();
}

#[cfg(all(feature = "multi-process", not(target_os = "linux")))]
pub fn lock_down_broker() {}

/// Cap the fork server (Linux only — it is the one platform with a zygote).
#[cfg(all(feature = "multi-process", target_os = "linux"))]
pub fn lock_down_fork_server() {
    imp::lock_down_fork_server();
}

/// Verify at startup that the fork-server filter permits what a forked
/// renderer needs on *this* host's C library, aborting if it does not. Called
/// straight after [`lock_down_fork_server`]. The allowlist is libc-sensitive in
/// ways a compile-time check cannot see, so this verifies rather than predicts
/// — see the backend for what varies and why.
#[cfg(all(feature = "multi-process", target_os = "linux"))]
pub fn verify_fork_server_filter() {
    imp::verify_fork_server_filter();
}

/// Test hook: run the canary against a filter with one syscall deliberately
/// removed, so the integration suite can prove the canary *detects* rather than
/// merely passes. Aborts the process, as a real canary failure would. Spawned
/// only by the `selftest` role.
#[cfg(all(feature = "multi-process", target_os = "linux"))]
pub fn canary_must_detect_a_missing_syscall() -> ! {
    imp::canary_must_detect_a_missing_syscall()
}
