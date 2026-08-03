//! Drives the network process end to end, from a binary that dispatches child
//! roles the way a real embedder does.

use gosub_interface::font_system::{Confinement, FontSystem};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::Arc;

const BODY: &str = "<html><head><title>through the net process</title></head><body>ok</body></html>";

/// Run a font scenario against the font system named by argv[2].
macro_rules! with_font_backend {
    ($scenario:ident) => {{
        let backend = std::env::args().nth(2).unwrap_or_else(|| "parley".into());
        match backend.as_str() {
            "parley" => $scenario::<gosub_fontmanager::ParleyFontSystem>(),
            "cosmic" => $scenario::<gosub_fontmanager::CosmicFontSystem>(),
            #[cfg(feature = "pango-fonts")]
            "pango" => $scenario::<gosub_fontmanager::PangoFontSystem>(),
            #[cfg(feature = "skia-fonts")]
            "skia" => $scenario::<gosub_fontmanager::SkiaFontSystem>(),
            other => {
                eprintln!(
                    "font backend {other:?} is not compiled into this harness; \
                     'parley' and 'cosmic' are always available, 'pango' and 'skia' \
                     need the engine features 'pango-fonts' / 'skia-fonts'"
                );
                2
            }
        }
    }};
}

fn main() {
    // First statement, exactly as the docs require of an embedder: in a child
    // this runs the role and exits, so nothing below executes there. Skipping it
    // is the mistake the `guard` scenario reproduces.
    if std::env::var_os("GOSUB_HARNESS_SKIP_DISPATCH").is_none() {
        gosub_engine::child_process::dispatch();
    }

    let scenario = std::env::args().nth(1).unwrap_or_default();
    let code = match scenario.as_str() {
        "direct" => direct(),
        "engine" => engine(),
        "guard" => guard(),
        "decode" => decode(),
        "decode-garbage" => decode_garbage(),
        "decode-many" => decode_many(),
        "fonts-under-lockdown" => with_font_backend!(fonts_under_lockdown),
        "webfont-under-lockdown" => with_font_backend!(webfont_under_lockdown),
        "fonts-under-font-readable-lockdown" => with_font_backend!(fonts_under_font_readable_lockdown),
        "webfont-under-font-readable-lockdown" => with_font_backend!(webfont_under_font_readable_lockdown),
        other => {
            eprintln!("unknown scenario {other:?}; expected 'direct' or 'engine'");
            2
        }
    };
    std::process::exit(code);
}

/// A 2x2 RGBA PNG: red, green, blue, white.
const SAMPLE_PNG: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00,
    0x02, 0x00, 0x00, 0x00, 0x02, 0x08, 0x06, 0x00, 0x00, 0x00, 0x72, 0xB6, 0x0D, 0x24, 0x00, 0x00, 0x00, 0x12, 0x49,
    0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0xF8, 0xCF, 0xC0, 0xF0, 0x1F, 0x0C, 0x81, 0x34, 0x18, 0x00, 0x00, 0x49, 0xC8,
    0x09, 0xF7, 0xF9, 0xAB, 0xB6, 0x0D, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
];

/// The decode boundary: real image bytes go into a throwaway process and the
/// exact pixels come back.
fn decode() -> i32 {
    use gosub_engine::decoder_process::client::ProcessImageDecoder;
    use gosub_interface::media_decoder::{BrokeredDecode, ImageDecoder};

    match ProcessImageDecoder.decode(Some("image/png"), SAMPLE_PNG) {
        Ok(BrokeredDecode::Raster(image)) => {
            if (image.width, image.height) != (2, 2) {
                eprintln!("expected a 2x2 image, got {}x{}", image.width, image.height);
                return 1;
            }
            let expected: &[u8] = &[255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255];
            if image.rgba.as_ref() != expected {
                eprintln!("pixels did not survive the boundary: {:?}", image.rgba.as_ref());
                return 1;
            }
            0
        }
        Ok(BrokeredDecode::Vector) => {
            eprintln!("a PNG should not decode as a vector");
            1
        }
        Err(e) => {
            eprintln!("decode in a separate process failed: {e}");
            1
        }
    }
}

/// The open question for a renderer process: can text be laid out by a process
/// confined the way a renderer must be?
fn fonts_under_lockdown<F: FontSystem + Default>() -> i32 {
    use gosub_interface::font_system::TextStyle;

    let mut fonts = F::default();
    println!("font backend: {}", std::any::type_name::<F>());

    // Warm-up. `families()` is documented to populate lazily-built databases, and
    // resolving plus shaping forces the actual file reads that follow.
    let families = fonts.families();
    println!("warm-up: {} families visible before lockdown", families.len());
    if families.is_empty() {
        eprintln!("no font families found; this host cannot answer the question");
        return 2;
    }
    // Exercise the trait hook rather than hand-rolled warm-up: this is what a
    // renderer will call, and it is the configured font system's own answer to
    // "get ready to be confined". Timed and measured, because the cost of that
    // answer is part of whether the strategy is usable. This scenario tests the
    // *full* lockdown, so any answer below `Full` ends it here — the tiered
    // scenarios cover the rest.
    let rss_before_mib = rss_mib();
    let start = std::time::Instant::now();
    match fonts.prepare_for_confinement() {
        Confinement::Full => {}
        other => {
            eprintln!("this font system does not support full confinement: {other:?}");
            return 3;
        }
    }
    println!(
        "prepare_for_confinement over {} families: {:?}, RSS {} -> {} MiB",
        families.len(),
        start.elapsed(),
        rss_before_mib,
        rss_mib()
    );

    gosub_sandbox::lock_down_renderer();

    // Text and a size never used above, so any per-face lazy load still pending
    // has to happen now — after the sandbox is in place.
    let cold_style = TextStyle::new("serif", 31.0);
    let (cold_w, cold_h) = fonts.measure("Text shaped only after the sandbox applied", &cold_style);
    if cold_w <= 0.0 || cold_h <= 0.0 {
        eprintln!("shaping under lockdown produced an empty box ({cold_w}x{cold_h})");
        return 1;
    }
    println!("shaped {cold_w:.1}x{cold_h:.1} under the renderer lockdown");
    0
}

/// The middle tier: can a font system that *cannot* be confined outright
/// (fontconfig consults the filesystem while shaping) run in a renderer that is
/// allowed to **read font paths and nothing else**?
fn fonts_under_font_readable_lockdown<F: FontSystem + Default>() -> i32 {
    use gosub_interface::font_system::TextStyle;

    let mut fonts = F::default();
    println!("font backend: {}", std::any::type_name::<F>());

    let families = fonts.families();
    println!("{} families visible before lockdown", families.len());
    if families.is_empty() {
        eprintln!("no font families found; this host cannot answer the question");
        return 2;
    }

    #[cfg(target_os = "linux")]
    {
        let paths = gosub_sandbox::font_filesystem_paths();
        if paths.is_empty() {
            eprintln!("no font paths exist on this host; the profile would test nothing");
            return 2;
        }
        println!(
            "font paths granted read-only: {}",
            paths
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        );
        let refs: Vec<(&std::path::Path, bool)> = paths.iter().map(|p| (p.as_path(), false)).collect();
        gosub_sandbox::lock_down_renderer_with_font_access(&refs);
    }
    #[cfg(not(target_os = "linux"))]
    {
        eprintln!("the font-readable renderer tier exists only on Linux");
        return 2;
    }

    // Never shaped, never warmed: the match runs cold, under the profile.
    let cold_style = TextStyle::new("serif", 31.0);
    let (cold_w, cold_h) = fonts.measure("Text shaped only after the sandbox applied", &cold_style);
    if cold_w <= 0.0 || cold_h <= 0.0 {
        eprintln!("shaping under the font-readable lockdown produced an empty box ({cold_w}x{cold_h})");
        return 1;
    }
    println!("shaped {cold_w:.1}x{cold_h:.1} under the font-readable renderer lockdown");
    0
}

/// Web fonts under the middle tier — the follow-up that decides whether the
/// tier is actually usable, because `@font-face` is everywhere.
fn webfont_under_font_readable_lockdown<F: FontSystem + Default>() -> i32 {
    use gosub_interface::font_system::{FontQuery, TextStyle};

    let mut fonts = F::default();
    println!("font backend: {}", std::any::type_name::<F>());
    let _ = fonts.families();

    let Ok(resolved) = fonts.resolve(&FontQuery::new(&["sans-serif"])) else {
        eprintln!("no resolvable font on this host to use as sample bytes");
        return 2;
    };
    let downloaded: Vec<u8> = resolved.blob.data.as_ref().as_ref().to_vec();
    if downloaded.is_empty() {
        eprintln!("resolved font carried no bytes; the control is broken");
        return 2;
    }
    println!("holding {} bytes of font data before lockdown", downloaded.len());

    #[cfg(target_os = "linux")]
    {
        let scratch = std::env::temp_dir().join(format!("gosub-webfont-scratch-{}", std::process::id()));
        if std::fs::create_dir_all(&scratch).is_err() {
            eprintln!("could not create the scratch directory; cannot set the tier up");
            return 2;
        }
        // Backends that stage fonts as files use the standard temp dir; point
        // it inside the ruleset so the write is scoped, not just allowed.
        std::env::set_var("TMPDIR", &scratch);

        let paths = gosub_sandbox::font_filesystem_paths();
        let mut refs: Vec<(&std::path::Path, bool)> = paths.iter().map(|p| (p.as_path(), false)).collect();
        refs.push((scratch.as_path(), true));
        gosub_sandbox::lock_down_renderer_with_font_access(&refs);
    }
    #[cfg(not(target_os = "linux"))]
    {
        eprintln!("the font-readable renderer tier exists only on Linux");
        return 2;
    }

    if let Err(e) = fonts.register_font(downloaded, Some("gosub-webfont-test")) {
        eprintln!("registering a web font under the font-readable lockdown failed: {e:?}");
        return 1;
    }
    let (w, h) = fonts.measure(
        "Web font registered after the sandbox applied",
        &TextStyle::new(resolved.family.clone(), 24.0),
    );
    if w <= 0.0 || h <= 0.0 {
        eprintln!("shaping with the registered font produced an empty box ({w}x{h})");
        return 1;
    }
    println!("registered and shaped {w:.1}x{h:.1} under the font-readable renderer lockdown");
    0
}

/// The follow-up question to the warm-up finding: a page can introduce a font at
/// any moment with `@font-face`, long after the sandbox is in place. Does that
/// need a file, and therefore a process that can open one?
fn webfont_under_lockdown<F: FontSystem + Default>() -> i32 {
    use gosub_interface::font_system::{FontQuery, TextStyle};

    let mut fonts = F::default();
    println!("font backend: {}", std::any::type_name::<F>());
    let _ = fonts.families();

    // Stand in for a downloaded font: bytes in hand, nothing else.
    let Ok(resolved) = fonts.resolve(&FontQuery::new(&["sans-serif"])) else {
        eprintln!("no resolvable font on this host to use as sample bytes");
        return 2;
    };
    let downloaded: Vec<u8> = resolved.blob.data.as_ref().as_ref().to_vec();
    if downloaded.is_empty() {
        eprintln!("resolved font carried no bytes; the control is broken");
        return 2;
    }
    println!("holding {} bytes of font data before lockdown", downloaded.len());

    // The renderer's documented sequence: prepare, confine, and only then let
    // content introduce fonts. Skipping the preparation here would test a
    // sequence no renderer runs — and shaping a web font still consults
    // fallback faces, which some backends (cosmic-text) load lazily per face.
    // Full lockdown only: backends answering a weaker tier are covered by the
    // font-readable scenarios.
    match fonts.prepare_for_confinement() {
        Confinement::Full => {}
        other => {
            eprintln!("this font system does not support full confinement: {other:?}");
            return 3;
        }
    }

    gosub_sandbox::lock_down_renderer();

    // Everything from here is what a renderer would do on encountering
    // `@font-face` mid-page.
    if let Err(e) = fonts.register_font(downloaded, Some("gosub-webfont-test")) {
        eprintln!("registering a web font under lockdown failed: {e:?}");
        return 1;
    }
    let (w, h) = fonts.measure(
        "Web font registered after the sandbox applied",
        &TextStyle::new(resolved.family.clone(), 24.0),
    );
    if w <= 0.0 || h <= 0.0 {
        eprintln!("shaping with the registered font produced an empty box ({w}x{h})");
        return 1;
    }
    println!("registered and shaped {w:.1}x{h:.1} under the renderer lockdown");
    0
}

/// Resident set size in MiB, from `/proc/self/statm` (pages), so the cost of a
/// strategy is a number rather than an impression. 0 where unavailable.
fn rss_mib() -> u64 {
    let Ok(statm) = std::fs::read_to_string("/proc/self/statm") else {
        return 0;
    };
    let pages: u64 = statm
        .split_whitespace()
        .nth(1)
        .and_then(|p| p.parse().ok())
        .unwrap_or(0);
    pages * 4096 / (1024 * 1024)
}

/// Decode the sample image repeatedly and report the wall-clock cost per image,
/// so the price of a process per decode is a measured number rather than a
/// guess. Count comes from argv[2], default 20.
fn decode_many() -> i32 {
    use gosub_engine::decoder_process::client::ProcessImageDecoder;
    use gosub_interface::media_decoder::ImageDecoder;

    let count: u32 = std::env::args().nth(2).and_then(|a| a.parse().ok()).unwrap_or(20);

    let start = std::time::Instant::now();
    for _ in 0..count {
        if ProcessImageDecoder.decode(Some("image/png"), SAMPLE_PNG).is_err() {
            eprintln!("decode failed during timing run");
            return 1;
        }
    }
    let elapsed = start.elapsed();
    println!(
        "{count} decodes in {:?} ({:.2} ms each)",
        elapsed,
        elapsed.as_secs_f64() * 1000.0 / f64::from(count)
    );
    0
}

/// Malformed input must come back as a refusal. This is the common case in the
/// wild — a truncated or hostile image — and it must not hang or crash the
/// broker.
fn decode_garbage() -> i32 {
    use gosub_engine::decoder_process::client::ProcessImageDecoder;
    use gosub_interface::media_decoder::ImageDecoder;

    // A PNG magic number followed by nonsense: it gets past a magic-byte sniff
    // and dies inside the decoder, which is where the danger actually lives.
    let mut bytes = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    bytes.extend(std::iter::repeat_n(0xA5, 4096));

    match ProcessImageDecoder.decode(Some("image/png"), &bytes) {
        Ok(other) => {
            eprintln!("garbage should not have decoded, got {other:?}");
            1
        }
        Err(_) => 0,
    }
}

/// An embedder that never dispatched: re-exec landed here, in `main`, rather
/// than in a component role. Spawning from this state would repeat the mistake
/// for every generation, so it must be refused.
fn guard() -> i32 {
    use gosub_engine::net::process::client::NetProcess;

    if !gosub_engine::child_process::is_child_process() {
        eprintln!("the guard scenario must be run with the child-role flag");
        return 2;
    }

    match NetProcess::spawn() {
        Ok(_) => {
            eprintln!("spawning should have been refused: an undispatched child must not spawn more");
            1
        }
        Err(e) => {
            // The message has to name the omission, or whoever hits this cannot
            // act on it.
            if e.to_string().contains("dispatch()") {
                0
            } else {
                eprintln!("refused, but not for the documented reason: {e}");
                1
            }
        }
    }
}

/// A one-shot HTTP server on an ephemeral port.
fn serve_once() -> std::io::Result<(u16, std::thread::JoinHandle<()>)> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();

    let handle = std::thread::spawn(move || {
        let Ok((mut stream, _)) = listener.accept() else {
            return;
        };
        let mut buf = [0u8; 4096];
        let _ = stream.read(&mut buf);
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{BODY}",
            BODY.len()
        );
        let _ = stream.write_all(response.as_bytes());
    });

    Ok((port, handle))
}

/// The transport on its own: does a request survive the round trip through a
/// separate, sandboxed process and come back intact?
fn direct() -> i32 {
    use gosub_engine::net::process::client::NetProcess;
    use gosub_engine::net::process::protocol::FetchOutcome;

    let Ok((port, server)) = serve_once() else {
        eprintln!("could not start the test server");
        return 1;
    };

    let net = match NetProcess::spawn() {
        Ok(n) => n,
        Err(e) => {
            eprintln!("could not spawn the network process: {e}");
            return 1;
        }
    };

    let outcome = net.fetch(
        format!("http://127.0.0.1:{port}/"),
        "GET".into(),
        vec![("accept".into(), "text/html".into())],
        None,
    );
    net.shutdown();
    drop(server);

    match outcome {
        FetchOutcome::Ok { status, body, .. } => {
            if status != 200 {
                eprintln!("expected status 200, got {status}");
                return 1;
            }
            if body != BODY.as_bytes() {
                eprintln!(
                    "body did not survive the round trip: {:?}",
                    String::from_utf8_lossy(&body)
                );
                return 1;
            }
            0
        }
        FetchOutcome::Error(e) => {
            eprintln!("fetch through the network process failed: {e}");
            1
        }
    }
}

/// The wiring: with isolation on, does an ordinary navigation still resolve —
/// through the child process rather than an in-process fetcher?
fn engine() -> i32 {
    use gosub_config::settings::Setting;
    use gosub_engine::events::{EngineEvent, NavigationEvent};
    use gosub_engine::storage::{InMemoryLocalStore, InMemorySessionStore, PartitionPolicy, StorageService};
    use gosub_engine::zone::ZoneServices;
    use gosub_engine::GosubEngine;
    use gosub_render_pipeline::render::backends::null::NullBackend;
    use gosub_render_pipeline::render::DefaultCompositor;

    let Ok((port, server)) = serve_once() else {
        eprintln!("could not start the test server");
        return 1;
    };

    let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("could not build a runtime: {e}");
            return 1;
        }
    };

    let code = runtime.block_on(async move {
        let mut engine: GosubEngine = GosubEngine::new(
            None,
            Arc::new(NullBackend::new()),
            Arc::new(DefaultCompositor::default()),
        );

        // Read once when the I/O runtime starts, so it must be set before start().
        if let Err(e) = engine.settings().set("security.network_process", Setting::Bool(true)) {
            eprintln!("could not enable process isolation: {e}");
            return 1;
        }

        let mut events = engine.subscribe_events();
        let Ok(run) = engine.start() else {
            eprintln!("engine failed to start");
            return 1;
        };
        tokio::spawn(run);

        let services = ZoneServices {
            storage: Arc::new(StorageService::new(
                Arc::new(InMemoryLocalStore::new()),
                Arc::new(InMemorySessionStore::new()),
            )),
            cookie_store: None,
            cookie_jar: None,
            partition_policy: PartitionPolicy::None,
        };
        let Ok(mut zone) = engine.create_zone(None, services, None) else {
            eprintln!("could not create a zone");
            return 1;
        };
        let Ok(tab) = zone.create_tab(Default::default(), None).await else {
            eprintln!("could not create a tab");
            return 1;
        };
        if tab.navigate(format!("http://127.0.0.1:{port}/")).await.is_err() {
            eprintln!("navigate failed");
            return 1;
        }

        // The navigation is only meaningful if it *finished*: a failure would
        // also end the wait, so the variant is what is asserted.
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                eprintln!("timed out waiting for the navigation to finish");
                return 1;
            }
            match tokio::time::timeout(remaining, events.recv()).await {
                Ok(Ok(EngineEvent::Navigation {
                    event: NavigationEvent::Finished { .. },
                    ..
                })) => break,
                Ok(Ok(EngineEvent::Navigation {
                    event: NavigationEvent::Failed { error, .. },
                    ..
                })) => {
                    eprintln!("navigation failed: {error}");
                    return 1;
                }
                Ok(Ok(_)) => continue,
                Ok(Err(e)) => {
                    eprintln!("event channel closed: {e}");
                    return 1;
                }
                Err(_) => {
                    eprintln!("timed out waiting for the navigation to finish");
                    return 1;
                }
            }
        }

        engine.close_zone(zone).await;
        let _ = engine.shutdown().await;
        0
    });

    drop(server);
    code
}
