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
        "engine" => engine(),
        "guard" => guard(),
        other => {
            eprintln!("unknown scenario {other:?}; expected 'direct', 'resolve', 'engine' or 'guard'");
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
