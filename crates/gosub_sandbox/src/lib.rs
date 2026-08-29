//! OS-level privilege capping for the engine and its child components.

// How a child process is created. Owned rather than delegated to
// `std::process::Command` because Windows access controls must be supplied at
// `CreateProcess` time - see the module docs.
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
/// hardening - warns rather than aborts on failure. Must be called *after*
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
    imp::apply_child_rlimits_with(DEFAULT_CHILD_DATA_LIMIT)
}

/// Committed-memory ceiling a child gets unless its profile says otherwise.
pub const DEFAULT_CHILD_DATA_LIMIT: u64 = 512 * 1024 * 1024;

/// [`apply_child_rlimits`] with a role-specific committed-memory ceiling.
#[cfg(feature = "multi-process")]
pub fn apply_child_rlimits_with(data_limit: u64) -> std::io::Result<()> {
    imp::apply_child_rlimits_with(data_limit)
}

/// Which namespaces a child is dropped into at spawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NamespaceIsolation {
    /// No unsharing at all.
    None,
    /// User, IPC and UTS namespaces only - the net component, which must keep
    /// the host network but has no business with its IPC or hostname.
    KeepNetwork,
    /// Empty network namespace (the load-bearing one) plus IPC and UTS as
    /// defense in depth, and (best-effort) a PID namespace - which, because
    /// the unshare is lazy, isolates the *fork server's forked renderers*
    /// rather than the caller.
    Full,
    /// [`Full`](NamespaceIsolation::Full) minus the PID namespace, for roles
    /// whose libraries must create threads: a process that has unshared
    /// its PID namespace cannot (measured - GLib/Pango aborts on it). Such
    /// roles never fork, so what the PID namespace would have isolated does
    /// not exist.
    NoPidNamespace,
}

/// Isolate a child's namespaces per `mode` (see [`NamespaceIsolation`]). The
/// mount namespace is deliberately left out for concrete reasons (see the
/// backend docs). Called from `pre_exec`, so it must stay async-signal-safe.
/// On platforms without namespaces this is deferred into the lockdown profile
/// - see the backend docs.
#[cfg(feature = "multi-process")]
pub fn isolate_namespaces(mode: NamespaceIsolation) -> std::io::Result<()> {
    imp::isolate_namespaces(mode)
}

/// Confine a renderer to pixels only: no network, no files, no new programs.
/// Called once the IPC link is connected. Fail-closed - the backend aborts the
/// process rather than let a renderer meant to be confined run unconfined.
#[cfg(feature = "multi-process")]
pub fn lock_down_renderer() {
    imp::lock_down_renderer();
}

/// Confine the image decoder: the renderer's pixels-only profile - no network,
/// no files, no new programs - reported under its own name, so the lockdown
/// banner says which component actually confined itself. Called once the IPC
/// link is connected. Fail-closed.
#[cfg(feature = "multi-process")]
pub fn lock_down_decoder() {
    imp::lock_down_decoder();
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

/// Confine the vault (cookie store): the bare content baseline (no network,
/// files, devices, or exec) plus non-dumpable. Linux only (the vault is
/// Linux-only). Fail-closed.
#[cfg(all(feature = "multi-process", target_os = "linux"))]
pub fn lock_down_vault() {
    imp::lock_down_vault();
}

/// Confine a renderer whose font system must read font files (fontconfig
/// stacks consult the filesystem while shaping; no warm-up covers it): the
/// renderer profile plus read-only, Landlock-scoped access to `fs_allow` -
/// pass [`font_filesystem_paths`]. Linux only. Fail-closed on the seccomp
/// install; the Landlock portion is best-effort like the other roles.
#[cfg(all(feature = "multi-process", target_os = "linux"))]
pub fn lock_down_renderer_with_font_access(fs_allow: &[(&std::path::Path, bool)]) {
    imp::lock_down_renderer_with_font_access(fs_allow);
}

/// The read-only paths [`lock_down_renderer_with_font_access`] normally wants:
/// font directories, fontconfig configuration, and its caches, filtered to
/// those that exist on this host. Linux only.
#[cfg(all(feature = "multi-process", target_os = "linux"))]
pub fn font_filesystem_paths() -> Vec<std::path::PathBuf> {
    imp::font_filesystem_paths()
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

/// Arm an alternate signal stack for the current thread, so the self-capturing
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

/// The AppContainer (lowbox) identity for a Windows child - the capability
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
/// also relabels it Low integrity) - the Windows analogue of the Linux services'
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

/// Confine the broker (engine) process with a loose sandbox: it must spawn
/// children, exec their libraries, thread, and open files and sockets, so it
/// gets a Landlock ruleset limiting writes to the temp dir (read/exec stay
/// open) plus a deny-list seccomp filter removing escalation syscalls
/// (`ptrace`, kernel-module/`kexec`/`bpf`, keyring, `mount`/`setns`, …).
/// Call on the main thread before the engine starts so every thread and child
/// inherits both. Linux only (a macOS Seatbelt broker profile is not built
/// yet). Best-effort: a kernel missing either mechanism leaves that layer off.
#[cfg(all(feature = "multi-process", target_os = "linux"))]
pub fn lock_down_broker() {
    imp::lock_down_broker();
}

#[cfg(all(feature = "multi-process", not(target_os = "linux")))]
pub fn lock_down_broker() {}

/// Cap the fork server (Linux only - it is the one platform with a zygote).
#[cfg(all(feature = "multi-process", target_os = "linux"))]
pub fn lock_down_fork_server() {
    imp::lock_down_fork_server();
}

/// Cap the fork server for a font system that needs the font-readable tier:
/// [`lock_down_fork_server`] plus the file-reading syscalls, with Landlock
/// scoping them to `fs_allow` - applied here, once, and inherited by every
/// forked renderer. Linux only. Fail-closed on the seccomp install.
#[cfg(all(feature = "multi-process", target_os = "linux"))]
pub fn lock_down_fork_server_with_font_access(fs_allow: &[(&std::path::Path, bool)]) {
    imp::lock_down_fork_server_with_font_access(fs_allow);
}

/// Cap a renderer forked from the fork server, to the tier its font system
/// answered: the renderer baseline, plus the file-reading syscalls when
/// `font_access` is set (path scoping was inherited from the fork server's
/// Landlock ruleset). Linux only. Fail-closed.
#[cfg(all(feature = "multi-process", target_os = "linux"))]
pub fn lock_down_forked_renderer(font_access: bool) {
    imp::lock_down_forked_renderer(font_access);
}

/// Which side of a [`fork_process`] call this process is. Linux only.
#[cfg(all(feature = "multi-process", target_os = "linux"))]
pub use imp::Forked;

/// Fork the calling process. The caller must be single-threaded - see the
/// backend for why the type system cannot enforce this. Linux only.
#[cfg(all(feature = "multi-process", target_os = "linux"))]
pub fn fork_process() -> std::io::Result<Forked> {
    imp::fork_process()
}

/// Wait for a forked child and return its raw wait status. Linux only.
#[cfg(all(feature = "multi-process", target_os = "linux"))]
pub fn reap_child(pid: i32) -> std::io::Result<i32> {
    imp::reap_child(pid)
}

/// Reap every forked child that has already exited, without blocking. For a
/// parent whose children live indefinitely (resident renderers) and die on
/// their own schedule. Linux only.
#[cfg(all(feature = "multi-process", target_os = "linux"))]
pub fn reap_exited_children() -> Vec<(i32, i32)> {
    imp::reap_exited_children()
}

/// Exit immediately without running destructors or `atexit` handlers - the only
/// correct way out of a forked child. Linux only.
#[cfg(all(feature = "multi-process", target_os = "linux"))]
pub fn exit_now(code: i32) -> ! {
    imp::exit_now(code)
}

/// Record where this process's argv lives, so [`set_process_title`] can
/// rewrite it later without a syscall. Must run before lockdown (it reads
/// `/proc`); forked children inherit the capture. A no-op off Linux, so
/// cross-platform child roles can call it unconditionally.
pub fn capture_process_title_region() {
    #[cfg(all(feature = "multi-process", target_os = "linux"))]
    imp::capture_process_title_region();
}

/// Rename this process in `ps`/`pstree`: the comm (15 bytes, pstree's default
/// display) plus the cmdline, when its region was captured pre-lockdown. Safe
/// under every filter here (`PR_SET_NAME` is on each allowlist). Names the
/// fork-without-exec children, which otherwise show their parent's identity,
/// and gives the exec'd roles a title that says what they are. A no-op off
/// Linux (other platforms show the executable name).
pub fn set_process_title(comm: &str, cmdline: &str) {
    #[cfg(all(feature = "multi-process", target_os = "linux"))]
    imp::set_process_title(comm, cmdline);
    #[cfg(not(all(feature = "multi-process", target_os = "linux")))]
    let _ = (comm, cmdline);
}

/// Keeps the fork server's PID namespace alive; see [`hold_pid_namespace_anchor`].
#[cfg(all(feature = "multi-process", target_os = "linux"))]
pub use imp::PidNamespaceAnchor;

/// Park a child as PID 1 of the fork server's (lazily-unshared) PID namespace,
/// for as long as the returned anchor lives - without it, the first exiting
/// child kills the namespace and every later `fork` fails with `ENOMEM`. Call
/// before the fork-server lockdown. Linux only.
#[cfg(all(feature = "multi-process", target_os = "linux"))]
pub fn hold_pid_namespace_anchor() -> std::io::Result<PidNamespaceAnchor> {
    imp::hold_pid_namespace_anchor()
}

/// In a forked child: close descriptors inherited from the parent that the
/// child must not hold (fork ignores `FD_CLOEXEC`). Call before lockdown.
#[cfg(all(feature = "multi-process", target_os = "linux"))]
pub fn close_inherited(fds: &[i32]) {
    imp::close_inherited(fds)
}

/// Verify at startup that the fork-server filter permits what a forked
/// renderer needs on *this* host's C library, aborting if it does not. Called
/// straight after [`lock_down_fork_server`]. The allowlist is libc-sensitive in
/// ways a compile-time check cannot see, so this verifies rather than predicts
/// - see the backend for what varies and why.
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
