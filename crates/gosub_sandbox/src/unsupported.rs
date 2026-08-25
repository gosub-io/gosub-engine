//! Fallback backend for platforms with no confinement mechanism wired up
//! (everything that is neither Linux nor macOS). The parent module only
//! compiles this file when neither `target_os = "linux"` nor
//! `target_os = "macos"` matches.

/// No sandbox mechanism here - run unconfined and be honest about it.
#[cfg(feature = "multi-process")]
pub fn lock_down_renderer() {
    eprintln!("[renderer] no sandbox on this platform — running unconfined");
}

#[cfg(feature = "multi-process")]
pub fn lock_down_decoder() {
    eprintln!("[decoder] no sandbox on this platform — running unconfined");
}

#[cfg(feature = "multi-process")]
pub fn lock_down_net(_fs_allow: &[(&std::path::Path, bool)]) {}

/// No confinement here either; the service runs unconfined like everything else
/// on this platform.
#[cfg(feature = "multi-process")]
pub fn lock_down_service(name: &str, _filesystem: bool, _device: bool, _fs_allow: &[(&std::path::Path, bool)]) {
    eprintln!("[{name}] no sandbox on this platform — running unconfined");
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
