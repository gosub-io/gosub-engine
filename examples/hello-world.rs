use cow_utils::CowUtils;
use gosub_engine::events::{MouseButton, NavigationEvent, ResourceEvent, TabCommand};
use gosub_engine::net::types::FetchResultMeta;
use gosub_engine::net::DecisionToken;
use gosub_engine::tab::{TabDefaults, TabHandle};
use gosub_engine::{
    cookies::DefaultCookieJar,
    events::EngineEvent,
    render::{DefaultCompositor, Viewport},
    storage::{InMemoryLocalStore, InMemorySessionStore, PartitionPolicy, StorageService},
    zone::ZoneConfig,
    zone::ZoneServices,
    Action, EngineConfig, EngineError, GosubEngine, NavigationId,
};
use http::header;
use std::sync::{Arc, RwLock};
use std::time::Duration;

fn init_tracing() {
    use tracing_subscriber::{fmt, layer::SubscriberExt, EnvFilter, Registry};

    let _ = tracing_log::LogTracer::init();

    // Default to warn. The tab worker emits a WARN for every unimplemented TabCommand
    // (e.g. SetTitle), which is expected during development — suppress it here so the example
    // output focuses on navigation and resource events.
    // Override via RUST_LOG, e.g. RUST_LOG=gosub_engine=debug for deeper inspection.
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("warn,gosub_engine::engine::tab::worker=error"));

    let fmt_layer = fmt::layer().with_target(true).with_level(true);

    let subscriber = Registry::default().with(filter).with(fmt_layer);
    let _ = tracing::subscriber::set_global_default(subscriber);
}

fn fmt_elapsed(d: std::time::Duration) -> String {
    if d.as_secs() >= 1 {
        format!("{:.2}s", d.as_secs_f64())
    } else {
        format!("{}ms", d.as_millis())
    }
}

fn short(id: &impl std::fmt::Display) -> String {
    let s = id.to_string();
    s.chars().take(8).collect()
}

#[tokio::main]
async fn main() -> Result<(), EngineError> {
    init_tracing();

    // Commented the panic, as this should not happen
    // panic("this will trigger a panic");

    // Configure the engine through the engine config builder. This will set up the main runtime
    // configuration of the engine. It's possible for some values to be changed at runtime, but
    // not all of them
    let engine_cfg = EngineConfig::builder()
        .max_zones(5)
        .build()
        .expect("Configuration is not valid");

    // Set up a render backend. In this example we use the NullBackend which does not render
    // anything.
    let backend = gosub_engine::render::backends::null::NullBackend::new().expect("null backend");

    // Instantiate and start the engine
    let mut engine = GosubEngine::new(
        Some(engine_cfg),
        Arc::new(backend),
        Arc::new(RwLock::new(DefaultCompositor::default())),
    );
    let engine_join_handle = engine.start().expect("cannot start engine");

    // Get our event channel to receive events from the engine. Note that you will only receive events
    // send from this point on.
    let mut event_rx = engine.subscribe_events();

    // Configure a zone. This works the same way as the engine config, using a builder
    // pattern to set up the configuration before building it.
    let zone_cfg = ZoneConfig::builder()
        .do_not_track(true)
        .accept_languages("fr-CH, fr;q=0.9, en;q=0.8, de;q=0.7, *;q=0.5")
        .build()
        .expect("ZoneConfig is not valid");

    // Create the services for this zone. These services are automatically provided to the tabs
    // created in the zone, but can be overridden on a per-tab basis if needed.
    let zone_services = ZoneServices {
        storage: Arc::new(StorageService::new(
            Arc::new(InMemoryLocalStore::new()),
            Arc::new(InMemorySessionStore::new()),
        )),
        cookie_store: None,
        cookie_jar: Some(DefaultCookieJar::new().into()),
        partition_policy: PartitionPolicy::None,
    };

    // Create the zone. Note that we can define our own zone ID to keep zones deterministic
    // (like a user profile), and we give the zone handle to the event channel so we can
    // receive events related to the zone.
    let mut zone = engine.create_zone(zone_cfg, zone_services, None)?;

    // Next, we create a tab in the zone. For now, we don't provide anything, but we should
    // be able to provide tab-specific services (like a different cookie jar, etc.)
    let def_values = TabDefaults {
        url: None,
        title: Some("New Tab".into()),
        viewport: Some(Viewport::new(0, 0, 800, 600)),
    };
    let tab = zone.create_tab(def_values, None).await.expect("cannot create tab");

    // From the tab handle, we can now send commands to the engine to control the tab.
    tab.send(TabCommand::SetViewport {
        x: 0,
        y: 0,
        width: 1024,
        height: 768,
    })
    .await?;

    // Navigate somewhere
    tab.send(TabCommand::Navigate {
        url: "https://news.ycombinator.com".into(),
    })
    .await?;

    // Simulate a little user input (mouse move + click at 100,100)
    tab.send(TabCommand::MouseMove { x: 100.0, y: 100.0 }).await?;
    tab.send(TabCommand::MouseDown {
        x: 100.0,
        y: 100.0,
        button: MouseButton::Left,
    })
    .await?;
    tab.send(TabCommand::MouseUp {
        x: 100.0,
        y: 100.0,
        button: MouseButton::Left,
    })
    .await?;

    // We can set metadata on the zone like this
    zone.set_title("My first Zone");
    zone.set_description("This is the new description");
    zone.set_color([255, 128, 64, 255]);

    // This is the application's main loop, where we receive events from the engine and
    // act on them. In a real application, you would probably want to run this in
    // a separate task/thread, and not block the main thread.

    let mut seen_intervals = 0usize;
    let mut interval = tokio::time::interval(Duration::from_secs(1));
    let tab_clone = tab.clone();

    loop {
        tokio::select! {
            Ok(ev) = event_rx.recv() => {
                handle_event(ev, tab_clone.clone()).await;
            }
            _ = tokio::signal::ctrl_c() => {
                println!("Shutting down...");
                break;
            }
            _ = interval.tick() => {
                seen_intervals += 1;
                if seen_intervals >= 1000 {
                    break;
                }
            }
        }
    }

    engine.shutdown().await?;

    // Wait for the engine task to finish
    if let Some(handle) = engine_join_handle {
        if let Err(join_err) = handle.await {
            eprintln!("engine task panicked: {join_err}");
        }
    }

    println!("Done. Exiting.");
    Ok(())
}

async fn on_decision_required(
    tab_handle: TabHandle,
    nav_id: NavigationId,
    meta: FetchResultMeta,
    decision_token: DecisionToken,
) {
    let ct: String = meta
        .headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_string();

    let action = if let Some(disp) = meta.headers.get(http::header::CONTENT_DISPOSITION) {
        let s = disp.to_str().unwrap_or_default().cow_to_ascii_lowercase();
        if s.contains("attachment") {
            Action::Download {
                dest: std::path::PathBuf::from("/tmp/downloaded.bin"),
            }
        } else {
            Action::Render
        }
    } else if ct.starts_with("text/html") || ct.starts_with("text/") || ct == "application/json" {
        Action::Render
    } else {
        Action::Download {
            dest: std::path::PathBuf::from("/tmp/downloaded.bin"),
        }
    };

    // Send back to the engine what we like to do with this navigation
    let _ = tab_handle
        .cmd_tx
        .send(TabCommand::SubmitDecision {
            nav_id,
            decision_token,
            action,
        })
        .await;
}

async fn handle_event(ev: EngineEvent, tab_handle: TabHandle) {
    match ev {
        EngineEvent::ZoneCreated { zone_id } => {
            println!("[zone] created   {}", short(&zone_id));
        }
        EngineEvent::TabCreated { tab_id, .. } => {
            println!("[tab ] created   {}", short(&tab_id));
        }

        EngineEvent::Navigation { tab_id, event } => {
            let t = short(&tab_id);
            match event {
                NavigationEvent::Started { url, .. } => {
                    println!("[nav ] →         [{t}] {url}");
                }
                NavigationEvent::Committed { url, .. } => {
                    println!("[nav ] committed [{t}] {url}");
                }
                NavigationEvent::Finished { url, .. } => {
                    println!("[nav ] finished  [{t}] {url}");
                }
                NavigationEvent::Failed { url, error, .. } => {
                    println!("[nav ] FAILED    [{t}] {url}  ({error})");
                }
                NavigationEvent::Cancelled { url, reason, .. } => {
                    println!("[nav ] cancelled [{t}] {url}  ({reason:?})");
                }
                NavigationEvent::FailedUrl { url, error, .. } => {
                    println!("[nav ] failed-url [{t}] {url}  ({error:?})");
                }
                NavigationEvent::Progress {
                    received_bytes,
                    expected_length,
                    elapsed,
                    ..
                } => {
                    let kb = received_bytes / 1024;
                    let total = expected_length
                        .map(|n| format!("{} KB", n / 1024))
                        .unwrap_or_else(|| "?".into());
                    println!("[nav ] progress  [{t}] {kb} KB / {total}  ({})", fmt_elapsed(elapsed));
                }
                NavigationEvent::DecisionRequired {
                    nav_id,
                    meta,
                    decision_token,
                } => {
                    // The engine fetched response headers and needs us to decide: render or download?
                    // We inspect content-type and content-disposition and reply with Action::Render or Action::Download.
                    println!("[nav ] decision  [{t}] {}", short(&nav_id));
                    if tab_id != tab_handle.tab_id {
                        eprintln!("[nav ] warning: DecisionRequired for unexpected tab {t}");
                        return;
                    }
                    on_decision_required(tab_handle, nav_id, meta, decision_token).await;
                }
            }
        }

        EngineEvent::Resource { tab_id, event } => {
            let t = short(&tab_id);
            match event {
                ResourceEvent::Queued {
                    url, kind, priority, ..
                } => {
                    println!("[res ] queued    [{t}] {kind:?} pri={priority}  {url}");
                }
                ResourceEvent::Started { url, .. } => {
                    println!("[res ] started   [{t}] {url}");
                }
                ResourceEvent::Redirected { from, to, status, .. } => {
                    println!("[res ] redirect  [{t}] {status}  {from}  →  {to}");
                }
                ResourceEvent::Headers {
                    url,
                    status,
                    content_type,
                    content_length,
                    ..
                } => {
                    let ct = content_type.as_deref().unwrap_or("-");
                    let cl = content_length
                        .map(|n| format!("{n} B"))
                        .unwrap_or_else(|| "unknown".into());
                    println!("[res ] headers   [{t}] {status}  {ct}  {cl}  {url}");
                }
                ResourceEvent::Progress {
                    received_bytes,
                    expected_length,
                    elapsed,
                    ..
                } => {
                    let kb = received_bytes / 1024;
                    let total = expected_length
                        .map(|n| format!("{} KB", n / 1024))
                        .unwrap_or_else(|| "?".into());
                    println!("[res ] progress  [{t}] {kb} KB / {total}  ({})", fmt_elapsed(elapsed));
                }
                ResourceEvent::Finished {
                    url,
                    received_bytes,
                    elapsed,
                    ..
                } => {
                    let kb = received_bytes as f64 / 1024.0;
                    let elapsed = elapsed.map(fmt_elapsed).unwrap_or_else(|| "-".into());
                    println!("[res ] finished  [{t}] {kb:.1} KB  {elapsed}  {url}");
                }
                ResourceEvent::Failed { url, error, .. } => {
                    println!("[res ] FAILED    [{t}] {url}  ({error})");
                }
                ResourceEvent::Cancelled { url, reason, .. } => {
                    println!("[res ] cancelled [{t}] {url}  ({reason:?})");
                }
            }
        }

        EngineEvent::Redraw { tab_id, .. } => {
            // With a real backend you'd composite a frame here.
            println!("[draw] frame     [{}]", short(&tab_id));
        }

        other => {
            println!("[?  ] {other:?}");
        }
    }
}
