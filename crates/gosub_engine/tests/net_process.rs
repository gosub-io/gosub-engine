//! Process isolation for the network stack, exercised for real.

// Nothing here exists without the machinery it drives; the single-process build
// has no child roles to test.
#![cfg(feature = "process-isolation")]
#![allow(clippy::expect_used)] // a harness that will not start must fail the test loudly

use std::process::Command;

fn harness() -> &'static str {
    env!("CARGO_BIN_EXE_net-process-harness")
}

fn run(scenario: &str) -> std::process::Output {
    Command::new(harness())
        .arg(scenario)
        .output()
        .expect("spawn net-process-harness")
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

/// The wiring: an ordinary navigation with `net.process_isolation` on resolves
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

/// An embedder that enables isolation without dispatching must be stopped, not
/// merely warned.
#[test]
fn an_undispatched_embedder_cannot_spawn_further_processes() {
    let out = Command::new(harness())
        .args(["guard", gosub_engine::child_process::ROLE_FLAG, "net"])
        .env("GOSUB_HARNESS_SKIP_DISPATCH", "1")
        .output()
        .expect("spawn net-process-harness");

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
        .expect("spawn net-process-harness");

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
