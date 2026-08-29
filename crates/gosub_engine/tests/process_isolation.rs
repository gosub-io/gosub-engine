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
/// syscalls the synthetic one in the proof of concept never made - reading the
/// trust store, resolving names, driving an async reactor - so a filter that is
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

/// A zone whose local store is a `FileLocalStore` is routed through the
/// storage process by default.
#[cfg(target_os = "linux")]
#[test]
fn a_zones_file_local_store_is_routed_through_the_storage_service() {
    let out = run("engine-storage-service");
    assert!(
        out.status.success(),
        "engine storage service scenario failed:\n{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

#[cfg(target_os = "linux")]
#[test]
fn local_storage_is_served_by_the_storage_service() {
    let out = run("storage");
    assert!(
        out.status.success(),
        "storage scenario failed:\n{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

/// The cookie vault on its own: jar that forwards, HttpOnly split, zone
/// partitioning, persistence brokered through a real SQLite store.
#[cfg(target_os = "linux")]
#[test]
fn the_cookie_vault_holds_partitioned_jars_and_persists_through_the_broker() {
    let out = run("vault");
    assert!(
        out.status.success(),
        "vault scenario failed:\n{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

/// A vault that dies is respawned on the next use: the zone comes back from
/// its store and the network process gets a new line, so the cookie still
/// reaches the next request.
#[cfg(target_os = "linux")]
#[test]
fn a_dead_cookie_vault_is_respawned_with_its_zones() {
    for mode in [
        &["engine-cookie-vault", "respawn"][..],
        &["engine-cookie-vault", "respawn", "in-process"][..],
    ] {
        let out = Command::new(harness())
            .args(mode)
            .output()
            .expect("spawn isolation-harness");
        assert!(
            out.status.success(),
            "{mode:?} failed:\n{}\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

/// With the vault and the network process on, a cookie a page sets reaches the
/// page's next request without the engine process ever attaching it.
#[cfg(target_os = "linux")]
#[test]
fn a_cookie_flows_from_the_vault_through_the_network_process() {
    let out = run("engine-cookie-vault");
    assert!(
        out.status.success(),
        "engine cookie vault scenario failed:\n{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

/// The same with in-process fetching: the broker's forwarding jar asks the vault.
#[cfg(target_os = "linux")]
#[test]
fn a_cookie_flows_from_the_vault_with_in_process_fetching() {
    let out = run_with_backend("engine-cookie-vault", "in-process");
    assert!(
        out.status.success(),
        "engine cookie vault (in-process) scenario failed:\n{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

/// A streamed body crosses the network-process boundary through a
/// shared-memory ring: head in-band, ring fd right behind it, bytes intact.
#[cfg(target_os = "linux")]
#[test]
fn a_body_streams_through_the_network_process() {
    let out = run("stream");
    assert!(
        out.status.success(),
        "streaming scenario failed:\n{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Resolving a real hostname inside the sandboxed network process: a
/// reserved `.invalid` name must fail to resolve without taking the process
/// down (the NSS `dlopen` and resolver syscalls that `127.0.0.1` never
/// exercises), the strict fetcher must refuse loopback, and the process must
/// still serve afterwards.
#[test]
fn the_network_process_survives_hostname_resolution() {
    let out = run("resolve");
    assert!(
        out.status.success(),
        "hostname resolution scenario failed:\n{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

/// A *spawned* child must run under the restricted token, not fall back to
/// the inherited one: `gosub_sandbox::spawn` reports the fallback on stderr,
/// which the harness (spawning the network process) inherits.
#[cfg(target_os = "windows")]
#[test]
fn spawned_children_get_a_restricted_token() {
    let out = run("direct");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("using inherited token"),
        "a child fell back to the inherited token - restricted_token() failed:\n{stderr}"
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
/// A renderer denies `openat` - that is most of what makes it a renderer - but
/// font stacks read font files lazily, on first use of a family. Measurement
/// showed the laziness is per family, not per shape: a family resolved and
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

/// The same confinement property, for the *other* always-compiled font system -
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

/// Web fonts after lockdown, for cosmic-text - the sequence a renderer actually
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

/// The fork server consumes the confinement answer end to end: for a
/// `Full`-tier font system it warms once, confines itself, and forks a
/// renderer that shapes under the strictest sandbox using only inherited,
/// copy-on-write font state.
#[cfg(target_os = "linux")]
#[test]
fn the_fork_server_forks_a_confined_renderer_for_a_full_tier_font_system() {
    let out = run("fork-server");

    if out.status.code() == Some(2) {
        eprintln!("skipping: {}", String::from_utf8_lossy(&out.stderr).trim());
        return;
    }
    assert!(
        out.status.success(),
        "the fork-server roundtrip failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("tier: Full"),
        "expected the default font system to announce the Full tier:\n{stdout}"
    );
}

/// The same roundtrip for the other always-compiled font system, whose warmed
/// state is the per-face override - a forked renderer shaping proves the
/// override's work really crosses the fork.
#[cfg(target_os = "linux")]
#[test]
fn the_fork_server_forks_a_confined_renderer_with_cosmic_text() {
    let out = run_with_backend("fork-server", "cosmic");

    if out.status.code() == Some(2) {
        eprintln!("skipping: {}", String::from_utf8_lossy(&out.stderr).trim());
        return;
    }
    assert!(
        out.status.success(),
        "the cosmic fork-server roundtrip failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Resident renderers: one process per (zone, site) shared by its tabs, a
/// cross-site navigation moving a tab to another process, and the last tab
/// leaving shutting the process down.
#[cfg(target_os = "linux")]
#[test]
fn resident_renderers_are_keyed_by_site_and_live_with_their_tabs() {
    let out = run("renderer-lifecycle");

    if out.status.code() == Some(2) {
        eprintln!("skipping: {}", String::from_utf8_lossy(&out.stderr).trim());
        return;
    }
    assert!(
        out.status.success(),
        "the resident renderer lifecycle failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// A resident renderer rasterizes around the viewport, extends on scroll and
/// evicts what falls far behind - the tile budget, out of process.
#[cfg(target_os = "linux")]
#[test]
fn a_resident_renderer_rasterizes_a_viewport_window_and_extends_on_scroll() {
    let out = run("renderer-scroll-window");

    if out.status.code() == Some(2) {
        eprintln!("skipping: {}", String::from_utf8_lossy(&out.stderr).trim());
        return;
    }
    assert!(
        out.status.success(),
        "the resident renderer's scroll window failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Hover on a retained page repaints the hovered element's tiles without a
/// parse or layout, and does nothing when the pointer stays put.
#[cfg(target_os = "linux")]
#[test]
fn a_resident_renderer_repaints_hover_without_relayout() {
    let out = run("renderer-hover");

    if out.status.code() == Some(2) {
        eprintln!("skipping: {}", String::from_utf8_lossy(&out.stderr).trim());
        return;
    }
    assert!(
        out.status.success(),
        "the resident renderer's hover repaint failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// A resident renderer that dies is replaced on the next request, and the
/// tab renders again in the replacement.
#[cfg(target_os = "linux")]
#[test]
fn a_dead_resident_renderer_is_replaced() {
    let out = run("renderer-crash");

    if out.status.code() == Some(2) {
        eprintln!("skipping: {}", String::from_utf8_lossy(&out.stderr).trim());
        return;
    }
    assert!(
        out.status.success(),
        "renderer crash recovery failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Through the engine: a renderer crash reaches the embedder as
/// `RendererCrashed`, and the tab comes back in a fresh process.
#[cfg(target_os = "linux")]
#[test]
fn a_renderer_crash_is_announced_and_the_tab_recovers() {
    let out = run("engine-renderer-crash");

    if out.status.code() == Some(2) {
        eprintln!("skipping: {}", String::from_utf8_lossy(&out.stderr).trim());
        return;
    }
    assert!(
        out.status.success(),
        "engine-level renderer crash handling failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// A remote render never waits for an image download: the page paints without
/// it and paints again once it has arrived.
#[cfg(target_os = "linux")]
#[test]
fn a_remote_render_does_not_wait_for_images() {
    let out = run("engine-renderer-slow-image");

    if out.status.code() == Some(2) {
        eprintln!("skipping: {}", String::from_utf8_lossy(&out.stderr).trim());
        return;
    }
    assert!(
        out.status.success(),
        "deferred image loading failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// A resident renderer over a long session: memory levels off across many
/// navigations, and tabs coming and going leave no zombies.
#[cfg(target_os = "linux")]
#[test]
fn a_resident_renderer_survives_a_long_session_without_growing() {
    let out = run("renderer-soak");

    if out.status.code() == Some(2) {
        eprintln!("skipping: {}", String::from_utf8_lossy(&out.stderr).trim());
        return;
    }
    assert!(
        out.status.success(),
        "the renderer soak failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// The render pipeline - parse, style, layout, layering, tiling, paint -
/// under the strictest renderer sandbox, in-process (no fork machinery), so a
/// pipeline-vs-sandbox regression is directly attributable.
#[cfg(target_os = "linux")]
#[test]
fn the_render_pipeline_runs_under_the_renderer_lockdown() {
    let out = run("render-under-lockdown");

    if out.status.code() == Some(2) {
        eprintln!("skipping: {}", String::from_utf8_lossy(&out.stderr).trim());
        return;
    }
    assert!(
        out.status.success(),
        "the pipeline under the renderer lockdown failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// The engine-side wiring: with `security.renderer_process` on, `start()`
/// spawns the fork server, announces its tier, renders through the
/// engine-held handle, and `shutdown()` tears it down cleanly.
#[cfg(target_os = "linux")]
#[test]
fn the_engine_spawns_the_renderer_fork_server_behind_its_setting() {
    let out = run("engine-renderer-process");

    if out.status.code() == Some(2) {
        eprintln!("skipping: {}", String::from_utf8_lossy(&out.stderr).trim());
        return;
    }
    assert!(
        out.status.success(),
        "the engine's renderer-process wiring failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("tier: Full"),
        "expected the default font system to announce the Full tier through the engine:\n{stdout}"
    );
}

/// The exec-fresh renderer: one throwaway, font-readable-confined process
/// renders one page - the `FontPathsReadable` tier's whole render path,
/// driven directly. Runs with the default (Full-tier) font system here,
/// which the weaker profile also serves; the tier-2 backends exercise the
/// same path behind their features.
#[cfg(target_os = "linux")]
#[test]
fn an_exec_fresh_renderer_renders_one_page_confined() {
    let out = run("exec-renderer");

    if out.status.code() == Some(2) {
        eprintln!("skipping: {}", String::from_utf8_lossy(&out.stderr).trim());
        return;
    }
    assert!(
        out.status.success(),
        "the exec'd renderer roundtrip failed:\n{}",
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
