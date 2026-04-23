use gosub_engine_api::events::{MouseButton, NavigationEvent, ResourceEvent, TabCommand};
use gosub_engine_api::net::types::FetchResultMeta;
use gosub_engine_api::net::DecisionToken;
use gosub_engine_api::tab::{TabDefaults, TabHandle};
use gosub_engine_api::{
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

fn init_json_tracing() {
    use tracing_subscriber::{fmt, layer::SubscriberExt, EnvFilter, Registry};

    let _ = tracing_log::LogTracer::init();

    log::set_max_level(log::LevelFilter::Trace);

    let filter = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new("info,gosub_engine=trace,gosub=trace"))
        .unwrap();

    let fmt_layer = fmt::layer()
        // .json()
        .with_target(true)
        .with_level(true)
        .with_thread_ids(true);

    let subscriber = Registry::default().with(filter).with(fmt_layer);
    let _ = tracing::subscriber::set_global_default(subscriber);
}

#[tokio::main]
async fn main() -> Result<(), EngineError> {
    // Allow debugging with tokio-console
    // console_subscriber::init();
    init_json_tracing();

    log::info!("Starting gosub engine...");

    // Configure the engine through the engine config builder. This will set up the main runtime
    // configuration of the engine. It's possible for some values to be changed at runtime, but
    // not all of them
    let engine_cfg = EngineConfig::builder()
        .max_zones(5)
        .build()
        .expect("Configuration is not valid");

    // Set up a render backend. In this example we use the NullBackend which does not render
    // anything.
    let backend = gosub_engine_api::render::backends::null::NullBackend::new().expect("null backend");

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
    let mut seen_frames = 0usize;
    let mut interval = tokio::time::interval(Duration::from_secs(1));
    let tab_clone = tab.clone();

    loop {
        tokio::select! {
            Ok(ev) = event_rx.recv() => {
                // println!("Received event: {:?}", ev);

                // Just count the frames we see for now
                if matches!(ev, EngineEvent::Redraw { .. }) {
                    seen_frames += 1;
                    println!("Total frames seen so far: {seen_frames}");
                }

                handle_event(ev, tab_clone.clone()).await;
            }
            _ = tokio::signal::ctrl_c() => {
                println!("Received Ctrl-C, shutting down...");
                break;
            }
            _ = interval.tick() => {
                println!("Ticking the UA interval");

                seen_intervals += 1;
                if seen_intervals >= 1000 {
                    println!("Seen {seen_intervals} intervals, exiting main loop");
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
        let s = disp.to_str().unwrap_or_default().to_ascii_lowercase();
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
        EngineEvent::TabCreated { tab_id, .. } => {
            // let tab = self.tabs.get(&tab_id).expect("Unknown tab");
            println!("[event] TabCreated: {tab_id:?}");
        }
        EngineEvent::Navigation { tab_id, event } => match event {
            NavigationEvent::DecisionRequired {
                nav_id,
                meta,
                decision_token,
            } => {
                // If we find a response meta event, we need to decide how to handle the response (saving / download, engine rendering etc.)
                println!("[event] DecisionRequest found: {tab_id} {nav_id} DecisionToken: {decision_token:?}");

                if tab_id != tab_handle.tab_id {
                    println!("Warning: DecisionRequired event for unknown tab_id: {tab_id}");
                    return;
                }

                // Normally, we should check if the tab_id we get actually matches one of our tabs.
                on_decision_required(tab_handle, nav_id, meta, decision_token).await;
            }

            NavigationEvent::Started { nav_id, url } => {
                println!("[event] NavigationStarted:\n     TabId: {tab_id}\n     NavId: {nav_id}\n     Url: {url}");
            }
            NavigationEvent::Committed { nav_id, url } => {
                println!("[event] NavigationCommitted:\n     TabId: {tab_id}\n     NavId: {nav_id}\n     Url: {url}");
            }
            NavigationEvent::Finished { nav_id, url } => {
                println!("[event] NavigationFinished:\n     TabId: {tab_id}\n     NavId: {nav_id}\n     Url: {url}");
            }
            NavigationEvent::Failed { nav_id, url, error } => {
                let nav_id = match nav_id {
                    Some(nav_id) => nav_id.to_string(),
                    None => "".into(),
                };

                println!("[event] NavigationFailed:\n     TabId: {tab_id}\n     NavId: {nav_id}\n     Url: {url}\n     Error: {error}");
            }
            NavigationEvent::Cancelled { nav_id, url, reason } => {
                println!("[event] NavigationCancelled:\n     TabId: {tab_id}\n     NavId: {nav_id}\n     Url: {url}\n     Reason: {reason:?}");
            }

            NavigationEvent::Progress {
                nav_id,
                received_bytes,
                expected_length,
                elapsed,
            } => {
                println!("[event] NavigationProgress:\n     TabId: {tab_id}\n     NavId: {nav_id}\n     Received Bytes: {received_bytes}\n     Expected Length: {expected_length:?}\n     Elapsed: {elapsed:?}");
            }
            NavigationEvent::FailedUrl { nav_id, url, error } => {
                println!("[event] NavigationFailedUrl:\n     TabId: {tab_id}\n     NavId: {nav_id:?}\n     Url: {url}\n     Error: {error:?}");
            }
        },
        EngineEvent::Resource { tab_id, event } => match event {
            ResourceEvent::Queued {
                request_id,
                reference,
                url,
                kind,
                initiator,
                priority,
            } => {
                println!("[event] ResourceQueued:\n     TabId: {tab_id}\n     ReqId: {request_id}\n     Ref: {reference}\n     Url: {url}\n     Kind: {kind:?}\n     Initator: {initiator:?}\n     Priority: {priority}");
            }
            ResourceEvent::Started {
                request_id,
                reference,
                url,
                kind,
                initiator,
            } => {
                println!("[event] ResourceStarted:\n     TabId: {tab_id}\n     ReqId: {request_id}\n      Ref: {reference}\n     Url: {url}\n     Kind: {kind:?}\n     Initator: {initiator:?}");
            }
            ResourceEvent::Redirected {
                request_id,
                reference,
                from,
                to,
                status,
            } => {
                println!("[event] ResourceRedirected:\n     TabId: {tab_id}\n     ReqId: {request_id}\n     Ref: {reference}\n     From: {from}\n     To: {to}\n     Status: {status}");
            }
            ResourceEvent::Progress {
                request_id,
                reference,
                received_bytes,
                expected_length,
                elapsed,
            } => {
                let el = expected_length.unwrap_or(0);
                println!("[event] ResourceProgress: \n     TabId: {tab_id}\n     ReqId: {request_id}\n     Ref: {reference}\n     Received Bytes: {received_bytes}\n     Expected Length: {el}\n     Elapsed: {elapsed:?}");
            }
            ResourceEvent::Finished {
                request_id,
                reference,
                url,
                received_bytes,
                elapsed,
            } => {
                // let content_type = content_type.unwrap_or_default();
                println!("[event] ResourceFinished:\n     TabId: {tab_id}\n     ReqId: {request_id}\n     Ref: {reference}\n     Url: {url}\n     Elapsed: {elapsed:?}\n     Received: {received_bytes}");
            }
            ResourceEvent::Failed {
                request_id,
                reference,
                url,
                error,
            } => {
                println!("[event] ResourceFailed:\n     TabId: {tab_id}\n     ReqId: {request_id}\n     Ref: {reference}\n     Url: {url}\n     Error: {error}");
            }
            ResourceEvent::Cancelled {
                request_id,
                reference,
                url,
                reason,
            } => {
                println!("[event] ResourceCancelled:\n     TabId: {tab_id}\n     ReqId: {request_id}\n     Ref: {reference}\n     Url: {url}\n     Reason: {reason:?}");
            }
            ResourceEvent::Headers {
                request_id,
                reference,
                url,
                status,
                content_length,
                content_type,
                headers,
            } => {
                let content_type = content_type.unwrap_or_default();
                let content_length = match content_length {
                    Some(len) => len.to_string(),
                    None => "unknown".into(),
                };
                println!("[event] ResourceHeaders:\n     TabId: {tab_id}\n     ReqId: {request_id}\n     Ref: {reference}\n     Url: {url}\n     Status: {status}\n     Content-Length: {content_length}\n     Content-Type: {content_type}\n     Headers:\n");
                for (k, v) in headers {
                    println!("         {k}: {v}");
                }
            }
        },
        EngineEvent::Redraw { tab_id, .. } => {
            // With a real backend, you might get a handle/texture to present here.
            println!("[event] FrameReady for tab={tab_id:?}");
        }
        other => {
            // Keep this to see what else your engine is emitting right now.
            println!("[event] {:?}", other);
        }
    }
}
