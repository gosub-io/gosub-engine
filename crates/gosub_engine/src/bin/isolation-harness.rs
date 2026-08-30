//! Drives the network process end to end, from a binary that dispatches child
//! roles the way a real embedder does.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::Arc;

const BODY: &str = "<html><head><title>through the net process</title></head>\
<body style=\"margin:0\"><a href=\"https://example.test/target\" \
style=\"display:block;width:400px;height:200px\">a link to hover</a></body></html>";

/// The harness's render configuration: null backend and compositor, nothing
/// composites here.
#[derive(Clone, Debug, PartialEq)]
struct TileConfig;

impl gosub_interface::config::ModuleConfiguration for TileConfig {
    type CssSystem = gosub_css3::system::Css3System;
    type Document = gosub_html5::document::document_impl::DocumentImpl<Self>;
    type HtmlParser = gosub_html5::parser::Html5Parser<'static, Self>;
}

impl gosub_engine::html::RenderConfiguration for TileConfig {
    type RenderBackend = gosub_render_pipeline::render::backends::null::NullBackend;
    type CompositorSink = gosub_render_pipeline::render::DefaultCompositor;
    type FontSystem = gosub_fontmanager::ParleyFontSystem;
}

fn main() {
    // First statement, exactly as the docs require of an embedder: in a child
    // this runs the role and exits, so nothing below executes there. Skipping it
    // is the mistake the `guard` scenario reproduces.
    if std::env::var_os("GOSUB_HARNESS_SKIP_DISPATCH").is_none() {
        gosub_engine::child_process::dispatch_with::<TileConfig>();
    }

    let scenario = std::env::args().nth(1).unwrap_or_default();
    let code = match scenario.as_str() {
        "direct" => direct(),
        "resolve" => resolve(),
        "decode" => decode(),
        "decode-garbage" => decode_garbage(),
        "decode-many" => decode_many(),
        "engine" => engine(),
        "guard" => guard(),
        other => {
            eprintln!("unknown scenario {other:?}; expected 'direct', 'resolve', 'engine', 'guard' or 'decode[-garbage|-many]'");
            2
        }
    };
    std::process::exit(code);
}

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

fn serve_once() -> std::io::Result<(u16, std::thread::JoinHandle<()>)> {
    serve_once_with(BODY)
}

/// A one-shot HTTP server on an ephemeral port, serving `body`.
fn serve_once_with(body: &'static str) -> std::io::Result<(u16, std::thread::JoinHandle<()>)> {
    serve_once_bytes(body.as_bytes().to_vec(), "text/html")
}

/// A one-shot HTTP server on an ephemeral port, serving `body` in small
/// writes with pauses, the way a body arrives over a real network.
fn serve_once_bytes(body: Vec<u8>, content_type: &'static str) -> std::io::Result<(u16, std::thread::JoinHandle<()>)> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();

    let handle = std::thread::spawn(move || {
        let Ok((mut stream, _)) = listener.accept() else {
            return;
        };
        let mut buf = [0u8; 4096];
        let _ = stream.read(&mut buf);
        let head = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let _ = stream.write_all(head.as_bytes());
        for chunk in body.chunks(32 * 1024) {
            if stream.write_all(chunk).is_err() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    });

    Ok((port, handle))
}

fn resolve() -> i32 {
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
    let Ok(runtime) = tokio::runtime::Builder::new_current_thread().enable_all().build() else {
        eprintln!("could not start a runtime");
        return 1;
    };
    let cancel = tokio_util::sync::CancellationToken::new();
    let fetch = |url: String| {
        let out = gosub_engine::net::process::client::Outbound::get(url);
        runtime.block_on(net.fetch(out, &cancel)).outcome
    };

    // 1. A name that cannot exist (RFC 2606).
    match fetch("http://gosub-hostname-probe.invalid/".into()) {
        FetchOutcome::Ok { status, .. } => {
            eprintln!("a .invalid name must not resolve, got status {status}");
            net.shutdown();
            return 1;
        }
        FetchOutcome::Error(e) => println!("resolution failed as it should: {e}"),
    }

    // 2. The process is still alive and serving.
    let outcome = fetch(format!("http://127.0.0.1:{port}/"));
    net.shutdown();
    drop(server);
    match outcome {
        FetchOutcome::Ok { status: 200, body, .. } if body == BODY.as_bytes() => {
            println!("network process survived resolution and still serves");
            0
        }
        other => {
            eprintln!("the network process did not survive: {other:?}");
            1
        }
    }
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
        }
        Err(e) => {
            eprintln!("decode in a separate process failed: {e}");
            return 1;
        }
    }

    // Header parsing runs in the child too.
    match ProcessImageDecoder.dimensions(Some("image/png"), SAMPLE_PNG) {
        Ok((2, 2)) => {}
        other => {
            eprintln!("expected 2x2 from the decoder's header parse, got {other:?}");
            return 1;
        }
    }

    // SVG comes back rasterized at its intrinsic size: the tree never leaves
    // the child. A logo-sized SVG (with text) must produce pixels, not a
    // dead decoder.
    match ProcessImageDecoder.decode(Some("image/svg+xml"), SAMPLE_SVG) {
        Ok(BrokeredDecode::Raster(image)) if image.width > 1 && image.height > 1 => 0,
        Ok(BrokeredDecode::Raster(image)) => {
            eprintln!("an SVG rasterized to {}x{}", image.width, image.height);
            1
        }
        Err(e) => {
            eprintln!("SVG decode in a separate process failed: {e}");
            1
        }
    }
}

/// A small SVG with a `<text>` element, so decoding it consults the fontdb.
const SAMPLE_SVG: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" width="18" height="18"><rect width="18" height="18" fill="#f60"/><text x="4" y="13" font-family="serif" font-size="10">Y</text></svg>"##;

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
/// wild - a truncated or hostile image - and it must not hang or crash the
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

    // `fetch` is async (the broker awaits it on its I/O runtime); the harness
    // has no runtime of its own, so give it a small one.
    let Ok(runtime) = tokio::runtime::Builder::new_current_thread().enable_all().build() else {
        eprintln!("could not start a runtime");
        return 1;
    };
    let cancel = tokio_util::sync::CancellationToken::new();
    let out = gosub_engine::net::process::client::Outbound {
        headers: vec![("accept".into(), "text/html".into())],
        ..gosub_engine::net::process::client::Outbound::get(format!("http://127.0.0.1:{port}/"))
    };
    let outcome = runtime.block_on(net.fetch(out, &cancel)).outcome;
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

/// The wiring: with isolation on, does an ordinary navigation still resolve -
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
            places: None,
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
