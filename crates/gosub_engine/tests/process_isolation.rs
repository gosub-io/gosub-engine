//! Process isolation, exercised for real: the network stack and the image
//! decoder each running in their own sandboxed process, plus the font property a
//! future renderer process depends on.

// Nothing here exists without the machinery it drives; the single-process build
// has no child roles to test.
#![cfg(feature = "process-isolation")]
#![allow(clippy::expect_used)] // a harness that will not start must fail the test loudly

use std::process::Command;

fn harness() -> &'static str {
    env!("CARGO_BIN_EXE_isolation-harness")
}

fn run(scenario: &str) -> std::process::Output {
    Command::new(harness())
        .arg(scenario)
        .output()
        .expect("spawn isolation-harness")
}

fn run_with_backend(scenario: &str, backend: &str) -> std::process::Output {
    Command::new(harness())
        .args([scenario, backend])
        .output()
        .expect("spawn isolation-harness")
}

/// The boundary itself: a request crosses into a separate, sandboxed process and
/// the response comes back byte-for-byte.
///
/// This is also the sandbox's own regression test. A real network stack needs
/// syscalls the synthetic one in the proof of concept never made — reading the
/// trust store, resolving names, driving an async reactor — so a filter that is
/// too tight shows up here as a `SIGSYS` rather than as a mysteriously failing
/// page much later.
#[test]
fn a_request_completes_through_the_network_process() {
    let out = run("direct");
    assert!(
        out.status.success(),
        "fetch through the network process failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// The wiring: an ordinary navigation with `security.network_process` on resolves
/// through the child rather than an in-process fetcher.
#[test]
fn a_navigation_resolves_with_process_isolation_enabled() {
    let out = run("engine");
    assert!(
        out.status.success(),
        "navigation under process isolation failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Image bytes cross into a throwaway, fully locked-down process and the exact
/// pixels come back.
#[test]
fn an_image_decodes_in_a_separate_process() {
    let out = run("decode");
    assert!(
        out.status.success(),
        "decoding in a separate process failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Malformed input comes back as a refusal, not an image and not a hang.
#[test]
fn a_malformed_image_is_refused_rather_than_decoded() {
    let out = run("decode-garbage");
    assert!(
        out.status.success(),
        "malformed image handling failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// A renderer process must be able to lay out text, and this pins the one
/// arrangement under which it can.
///
/// A renderer denies `openat` — that is most of what makes it a renderer — but
/// font stacks read font files lazily, on first use of a family. Measurement
/// showed the laziness is **per family, not per shape**: a family resolved and
/// shaped before the sandbox is applied goes on shaping new text at new sizes
/// afterwards, while one first touched under the sandbox dies on `SIGSYS`.
#[test]
fn a_warmed_font_system_can_shape_under_the_renderer_lockdown() {
    let out = run("fonts-under-lockdown");

    // Exit 2 means the host has no fonts to test with, which is a skip rather
    // than a failure: the property is about the sandbox, not the machine.
    if out.status.code() == Some(2) {
        eprintln!("skipping: {}", String::from_utf8_lossy(&out.stderr).trim());
        return;
    }
    assert!(
        out.status.success(),
        "a warmed font system should still shape once confined:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// A font that arrives *after* the sandbox is applied still works, as long as it
/// arrives as bytes.
#[test]
fn a_web_font_can_be_registered_under_the_renderer_lockdown() {
    let out = run("webfont-under-lockdown");

    if out.status.code() == Some(2) {
        eprintln!("skipping: {}", String::from_utf8_lossy(&out.stderr).trim());
        return;
    }
    assert!(
        out.status.success(),
        "registering a web font once confined should work:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// The same confinement property, for the *other* always-compiled font system —
/// because the engine is generic over font systems, the property is per
/// implementation, not per engine.
///
/// cosmic-text loads face data lazily per (face, weight), and shaping consults
/// fallback faces a family-by-family warm-up never touches; the trait's default
/// `prepare_for_confinement` measurably left it dying on `openat` under the
/// sandbox. This pins its override, which loads every face in the database.
#[test]
fn cosmic_text_can_shape_under_the_renderer_lockdown() {
    let out = run_with_backend("fonts-under-lockdown", "cosmic");

    if out.status.code() == Some(2) {
        eprintln!("skipping: {}", String::from_utf8_lossy(&out.stderr).trim());
        return;
    }
    assert!(
        out.status.success(),
        "a prepared cosmic-text font system should still shape once confined:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Web fonts after lockdown, for cosmic-text — the sequence a renderer actually
/// runs is prepare, confine, then let content register fonts, and for
/// cosmic-text the preparation is load-bearing even for a font that arrives as
/// bytes, because shaping it still consults fallback faces.
#[test]
fn a_web_font_can_be_registered_with_cosmic_text_under_the_renderer_lockdown() {
    let out = run_with_backend("webfont-under-lockdown", "cosmic");

    if out.status.code() == Some(2) {
        eprintln!("skipping: {}", String::from_utf8_lossy(&out.stderr).trim());
        return;
    }
    assert!(
        out.status.success(),
        "registering a web font once confined should work with cosmic-text:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// The middle sandbox tier: a renderer allowed to *read font paths and nothing
/// else* (plus one private writable scratch), for font systems that consult
/// the filesystem while shaping and so can never satisfy full confinement.
#[cfg(target_os = "linux")]
#[test]
fn a_font_system_can_shape_under_the_font_readable_lockdown() {
    let out = run("fonts-under-font-readable-lockdown");

    if out.status.code() == Some(2) {
        eprintln!("skipping: {}", String::from_utf8_lossy(&out.stderr).trim());
        return;
    }
    assert!(
        out.status.success(),
        "shaping under the font-readable renderer profile failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Web fonts under the middle tier, including the writable-scratch arrangement
/// some backends need (Pango stages a web font as a file; fontconfig's
/// app-font API takes a path, not memory).
#[cfg(target_os = "linux")]
#[test]
fn a_web_font_can_be_registered_under_the_font_readable_lockdown() {
    let out = run("webfont-under-font-readable-lockdown");

    if out.status.code() == Some(2) {
        eprintln!("skipping: {}", String::from_utf8_lossy(&out.stderr).trim());
        return;
    }
    assert!(
        out.status.success(),
        "registering a web font under the font-readable renderer profile failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// An embedder that enables isolation without dispatching must be stopped, not
/// merely warned.
#[test]
fn an_undispatched_embedder_cannot_spawn_further_processes() {
    let out = Command::new(harness())
        .args(["guard", gosub_engine::child_process::ROLE_FLAG, "net"])
        .env("GOSUB_HARNESS_SKIP_DISPATCH", "1")
        .output()
        .expect("spawn isolation-harness");

    assert!(
        out.status.success(),
        "spawning should have been refused with a message naming dispatch():\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// An embedder that does not dispatch child roles must not silently run the
/// engine's startup in what was supposed to be a network process.
#[test]
fn an_unknown_child_role_is_refused() {
    let out = Command::new(harness())
        .args([gosub_engine::child_process::ROLE_FLAG, "no-such-role"])
        .output()
        .expect("spawn isolation-harness");

    assert!(
        !out.status.success(),
        "an unknown role should not be treated as success"
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("unknown child role"),
        "expected the refusal to name the problem, got: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}
