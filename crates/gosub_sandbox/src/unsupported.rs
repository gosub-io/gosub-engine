//! Fallback backend for platforms with no confinement mechanism wired up
//! (everything that is not Linux, macOS or Windows). The parent module only
//! compiles this file when none of `target_os = "linux"`, `"macos"` and
//! `"windows"` matches.

/// No sandbox mechanism here, so a child does not run: it would hold every
/// right the embedder has while parsing what a page sent. The engine starts no
/// children on this platform (see [`crate::CONFINES_CHILDREN`]); this is the
/// backstop for anything that spawns one anyway.
#[cfg(feature = "multi-process")]
fn refuse_unconfined(role: &str) -> ! {
    eprintln!("[{role}] no sandbox on this platform - refusing to run unconfined");
    std::process::exit(1);
}

#[cfg(feature = "multi-process")]
pub fn lock_down_renderer() {
    refuse_unconfined("renderer");
}

#[cfg(feature = "multi-process")]
pub fn lock_down_decoder() {
    refuse_unconfined("decoder");
}

#[cfg(feature = "multi-process")]
pub fn lock_down_net() {
    refuse_unconfined("net");
}

#[cfg(feature = "multi-process")]
pub fn lock_down_service(name: &str, _filesystem: bool, _device: bool, _fs_allow: &[(&std::path::Path, bool)]) {
    refuse_unconfined(name);
}

/// rlimits are POSIX, but this fallback keeps the whole backend as no-ops so a
/// port is an all-or-nothing, clearly-visible piece of work rather than a
/// partial illusion of confinement.
/// The committed-memory ceiling has no per-process form here; see [`apply_child_rlimits`].
#[cfg(feature = "multi-process")]
pub fn apply_child_rlimits_with(_data_limit: u64) -> std::io::Result<()> {
    apply_child_rlimits()
}

#[cfg(feature = "multi-process")]
pub fn apply_child_rlimits() -> std::io::Result<()> {
    Ok(())
}

/// No network namespaces; nothing to do (see the Linux/macOS backends).
#[cfg(feature = "multi-process")]
pub fn isolate_namespaces(_mode: crate::NamespaceIsolation) -> std::io::Result<()> {
    Ok(())
}

/// No anti-debugging primitive wired up here.
pub fn deny_debugger_attach() {}

#[cfg(feature = "multi-process")]
pub fn apply_child_file_size_limit(_bytes: u64) -> std::io::Result<()> {
    Ok(())
}

#[cfg(feature = "multi-process")]
pub fn mark_all_fds_close_on_exec() {}
