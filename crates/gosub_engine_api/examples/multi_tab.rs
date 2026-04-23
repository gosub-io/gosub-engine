use gosub_engine_api::{
    cookies::DefaultCookieJar,
    events::EngineEvent,
    render::{DefaultCompositor, Viewport},
    storage::{InMemoryLocalStore, InMemorySessionStore, PartitionPolicy, StorageService},
    zone::ZoneConfig,
    zone::ZoneServices,
    EngineConfig, EngineError, GosubEngine,
};

use gosub_engine_api::events::{NavigationEvent, ResourceEvent};
use gosub_engine_api::tab::{TabDefaults, TabId};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use rand::prelude::IndexedRandom;
use rand::rng;
use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tokio::time::sleep;

static UI: Lazy<Mutex<Ui>> = Lazy::new(|| Mutex::new(Ui::new()));

struct Ui {
    mp: MultiProgress,
    bars: HashMap<TabId, ProgressBar>,
}

impl Ui {
    fn new() -> Self {
        Self {
            mp: MultiProgress::new(),
            bars: HashMap::new(),
        }
    }

    fn bar_for(&mut self, tab: TabId) -> ProgressBar {
        use std::collections::hash_map::Entry;
        match self.bars.entry(tab) {
            Entry::Occupied(e) => e.get().clone(),
            Entry::Vacant(v) => {
                let pb = self.mp.add(ProgressBar::new_spinner());
                pb.enable_steady_tick(Duration::from_millis(120));
                pb.set_style(
                    ProgressStyle::with_template("[{prefix:.dim}] {wide_msg}")
                        .unwrap()
                        .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏ "),
                );
                pb.set_prefix(format!("{tab}"));
                v.insert(pb.clone());
                pb
            }
        }
    }

    fn update(&mut self, tab: TabId, msg: impl Into<String>) {
        self.bar_for(tab).set_message(msg.into());
    }

    fn done(&mut self, tab: TabId) {
        if let Some(pb) = self.bars.remove(&tab) {
            pb.finish_and_clear();
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), EngineError> {
    console_subscriber::init();

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
        .max_tabs(100)
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

    let mut tab_id_to_idx: HashMap<TabId, usize> = HashMap::new();

    // Next, we create a tab in the zone. For now, we don't provide anything, but we should
    // be able to provide tab-specific services (like a different cookie jar, etc.)
    let mut tabs = Vec::new();
    for i in 0..25 {
        let def_values = TabDefaults {
            url: None,
            title: Some(format!("Tab {i}")),
            viewport: Some(Viewport::new(0, 0, 800, 600)),
        };

        let tab = zone
            .create_tab(def_values.clone(), None)
            .await
            .expect("cannot create tab");

        tab_id_to_idx.insert(tab.tab_id, i);
        tabs.push(tab);
    }

    #[allow(unused)]
    let autoexec_handle = tokio::spawn(async move {
        let domains: Vec<&'static str> = vec![
            "google.com",
            "bing.com",
            "yahoo.com",
            "wikipedia.org",
            "archive.org",
            "rust-lang.org",
            "github.com",
            "stackoverflow.com",
            "news.ycombinator.com",
            "x.com",
            "twitter.com",
            "facebook.com",
            "instagram.com",
            "linkedin.com",
            "tiktok.com",
            "reddit.com",
            "youtube.com",
            "netflix.com",
            "spotify.com",
            "imdb.com",
            "amazon.com",
            "apple.com",
            "microsoft.com",
            "cloudflare.com",
            "nytimes.com",
            "bbc.co.uk",
            "theguardian.com",
            "reuters.com",
            "bloomberg.com",
            "cnn.com",
            "arstechnica.com",
            "theverge.com",
        ];

        loop {
            let tab = {
                let mut rng = rng();
                tabs.choose(&mut rng).unwrap()
            };

            let domain = {
                let mut rng = rng();
                let dn = *domains.choose(&mut rng).unwrap();
                format!("https://{dn}")
            };

            tab.navigate(domain).await.unwrap();
            sleep(Duration::from_secs(1)).await;
        }
    });

    // This is the application's main loop, where we receive events from the engine and
    // act on them. In a real application, you would probably want to run this in
    // a separate task/thread, and not block the main thread.

    let mut seen_intervals = 0usize;
    let mut seen_frames = 0usize;
    let mut interval = tokio::time::interval(Duration::from_secs(1));

    loop {
        tokio::select! {
            Ok(ev) = event_rx.recv() => {
                // Just count the frames we see for now
                if matches!(ev, EngineEvent::Redraw { .. }) {
                    seen_frames += 1;
                }
                handle_event(ev);
            }
            _ = tokio::signal::ctrl_c() => {
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

    println!("Done. Exiting. Seen frames: {}", seen_frames);
    Ok(())
}

fn handle_event(ev: EngineEvent) {
    match ev {
        EngineEvent::TabCreated { tab_id, .. } => {
            UI.lock().update(tab_id, "created");
        }
        // EngineEvent::Load { tab_id, event } => {
        //     let mut ui = UI.lock();
        //     match event {
        //         LoadEvent::Started { url, .. } => ui.update(tab_id, format!("load: [STAR] {:20}", url)),
        //         LoadEvent::Progress {
        //             url,
        //             finished,
        //             bytes_received,
        //             ttfb,
        //             ..
        //         } => ui.update(
        //             tab_id,
        //             format!(
        //                 "load: [DOWN] {:20} TTFB: {ttfb} FIN: {finished} RX: {bytes_received}",
        //                 url
        //             ),
        //         ),
        //         LoadEvent::Finished { url, bytes, .. } => ui.update(tab_id, format!("load: [FINI] {:20} {bytes}", url)),
        //         LoadEvent::Failed { url, error, .. } => ui.update(tab_id, format!("load: [FAIL] {:20} {error}", url)),
        //         LoadEvent::Cancelled { url, reason, .. } => {
        //             ui.update(tab_id, format!("load: [CNCL] {:20} {reason}", url))
        //         }
        //     }
        // }
        EngineEvent::Navigation { tab_id, event } => {
            let mut ui = UI.lock();
            match event {
                NavigationEvent::Started { url, .. } => ui.update(tab_id, format!("nav: → {url}")),
                NavigationEvent::Committed { url, .. } => ui.update(tab_id, format!("nav: committed {url}")),
                NavigationEvent::Finished { url, .. } => ui.update(tab_id, format!("nav: finished {url}")),
                NavigationEvent::Failed { url, error, .. } => ui.update(tab_id, format!("nav: FAILED {url} ({error})")),
                NavigationEvent::Cancelled { url, reason, .. } => {
                    ui.update(tab_id, format!("nav: cancelled {url} [{reason:?}]"))
                }
                NavigationEvent::Progress {
                    received_bytes,
                    expected_length,
                    elapsed,
                    ..
                } => {
                    ui.update(
                        tab_id,
                        format!(
                            "nav: progress: {} {} {:?}",
                            received_bytes,
                            expected_length.unwrap_or(0),
                            elapsed
                        ),
                    );
                }
                NavigationEvent::FailedUrl { url, error, .. } => {
                    ui.update(tab_id, format!("nav: failed: {} {}", url, error));
                }
                NavigationEvent::DecisionRequired { nav_id, meta, decision_token } => {
                    println!("nav: decision required: {} {:?} {:?}", nav_id, meta, decision_token);
                }
            }
        }
        EngineEvent::Resource { tab_id, event } => {
            // Build a compact one-line summary per event
            let mut ui = UI.lock();
            match event {
                ResourceEvent::Queued { kind, priority, .. } => {
                    ui.update(tab_id, format!("res: queued {kind:?} pri={priority}"))
                }
                ResourceEvent::Started { url, .. } => ui.update(tab_id, format!("res: started {url}")),
                ResourceEvent::Redirected { from, to, status, .. } => {
                    ui.update(tab_id, format!("res: redirect {status} {from} → {to}"))
                }
                ResourceEvent::Progress { received_bytes, .. } => {
                    ui.update(tab_id, format!("res: {received_bytes} bytes…"))
                }
                ResourceEvent::Headers {
                    status,
                    content_length,
                    content_type,
                    ..
                } => {
                    let mut s = String::new();
                    let _ = write!(
                        s,
                        "res: headers {status} len={} type={}",
                        content_length
                            .map(|n| n.to_string())
                            .unwrap_or_else(|| "unknown".into()),
                        content_type.unwrap_or_default()
                    );
                    ui.update(tab_id, s);
                }
                ResourceEvent::Finished {
                    url,
                    received_bytes,
                    elapsed,
                    ..
                } => {
                    let kb = received_bytes as f64 / 1024.0;
                    ui.update(
                        tab_id,
                        format!("res: finished {} {:.2}Kb ({:?})", url, kb, elapsed),
                    )
                }
                ResourceEvent::Failed { url, error, .. } => ui.update(tab_id, format!("res: FAILED {url} ({error})")),
                ResourceEvent::Cancelled { url, reason, .. } => {
                    ui.update(tab_id, format!("res: cancelled {url} [{reason:?}]"))
                }
            }
        }
        EngineEvent::Redraw { tab_id, .. } => {
            UI.lock().update(tab_id, "frame ready");
        }
        EngineEvent::TabClosed { tab_id, .. } => {
            UI.lock().done(tab_id);
        }
        _other => {
            // UI.lock().update(TabId(0), format!("{other:?}")); // or ignore/log elsewhere
        }
    }
}
