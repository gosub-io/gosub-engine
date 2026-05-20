//! A minimal browser window that renders pages through the new gosub_pipeline render system.
//!
//! Usage:  cargo run --example pipeline-browser -- https://example.com
//!
//! The binary uses the full GosubEngine zone/tab/net API and routes rendering through the
//! 7-stage pipeline (rendertree → layout → layering → tiling → painting → rasterize →
//! composite) backed by Cairo.  The result is displayed in a GTK4 window.

use gosub_engine::cookies::SqliteCookieStore;
use gosub_engine::events::{EngineEvent, TabCommand};
use gosub_engine::render::backend::ExternalHandle;
use gosub_engine::render::{DefaultCompositor, Viewport};
use gosub_engine::storage::{InMemorySessionStore, PartitionPolicy, SqliteLocalStore, StorageService};
use gosub_engine::tab::{TabDefaults, TabId};
use gosub_engine::zone::{ZoneConfig, ZoneId, ZoneServices};
use gosub_engine::GosubEngine;
use gtk4::glib;
use gtk4::prelude::*;
use gtk4::{Application, ApplicationWindow, Box as GtkBox, DrawingArea, Entry, Orientation};
use once_cell::sync::Lazy;
use parking_lot::RwLock;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use tokio::runtime::{Builder, Runtime};
use tokio::sync::mpsc;
use url::Url;
use uuid::uuid;

const DEFAULT_ZONE: uuid::Uuid = uuid!("f1234567-abcd-4000-8000-000000000001");

static TOKIO_RT: Lazy<Runtime> = Lazy::new(|| {
    Builder::new_multi_thread()
        .enable_io()
        .enable_time()
        .thread_name("gosub-pipeline-rt")
        .build()
        .expect("tokio runtime")
});

fn main() {
    simple_logger::init_with_env().unwrap_or_default();

    let initial_url: Option<String> = std::env::args().nth(1);

    let app = Application::builder()
        .application_id("io.gosub.pipeline-browser")
        .build();

    app.connect_activate(move |app| {
        let _rt_guard = TOKIO_RT.enter();

        // Channel from engine → GTK: request a redraw
        let (tx_redraw, mut rx_redraw) = mpsc::unbounded_channel::<()>();

        let compositor = Arc::new(RwLock::new(DefaultCompositor::new({
            let tx = tx_redraw.clone();
            move || {
                let _ = tx.send(());
            }
        })));

        let backend = gosub_engine::render::backends::cairo::CairoBackend::new();
        let mut engine = GosubEngine::new(None, Arc::new(backend), compositor.clone());
        let _join = engine.start().expect("engine start");
        let event_rx = engine.subscribe_events();

        let zone_cfg = ZoneConfig::builder()
            .do_not_track(true)
            .build()
            .expect("ZoneConfig");

        let cookie_store: gosub_engine::cookies::CookieStoreHandle =
            SqliteCookieStore::new(".pipeline-browser-cookies.db".into()).into();

        let zone_services = ZoneServices {
            storage: Arc::new(StorageService::new(
                Arc::new(SqliteLocalStore::new(".pipeline-browser-local.db").expect("local store")),
                Arc::new(InMemorySessionStore::new()),
            )),
            cookie_store: Some(cookie_store),
            cookie_jar: None,
            partition_policy: PartitionPolicy::None,
        };

        let zone = Rc::new(RefCell::new(
            engine
                .create_zone(zone_cfg, zone_services, Some(ZoneId::from(DEFAULT_ZONE)))
                .expect("create_zone"),
        ));

        let tab = {
            let mut z = zone.borrow_mut();
            TOKIO_RT
                .block_on(z.create_tab(
                    TabDefaults {
                        url: None,
                        title: Some("Pipeline Browser".to_string()),
                        viewport: Some(Viewport::new(0, 0, 1024, 768)),
                    },
                    None,
                ))
                .expect("create_tab")
        };

        let tab_id: TabId = tab.tab_id;

        if let Some(url_str) = &initial_url {
            let mut s = url_str.clone();
            if !s.starts_with("http://") && !s.starts_with("https://") {
                s = format!("https://{s}");
            }
            if let Ok(url) = Url::parse(&s) {
                TOKIO_RT.block_on(async {
                    let _ = tab.send(TabCommand::Navigate { url: url.to_string() }).await;
                    let _ = tab.send(TabCommand::ResumeDrawing { fps: 30 }).await;
                });
            }
        }

        // Wrap the tab in Rc<RefCell<>> so closures can share it
        let tab = Rc::new(RefCell::new(tab));

        // --- Widgets ---
        let address_entry = Entry::new();
        address_entry.set_placeholder_text(Some("Enter URL…"));
        address_entry.set_hexpand(true);

        let drawing_area = DrawingArea::new();
        drawing_area.set_content_width(1024);
        drawing_area.set_content_height(768);
        drawing_area.set_vexpand(true);
        drawing_area.set_hexpand(true);
        drawing_area.set_focusable(true);

        // Forward engine redraw requests to GTK
        {
            let da = drawing_area.clone();
            glib::spawn_future_local(async move {
                while let Some(()) = rx_redraw.recv().await {
                    da.queue_draw();
                }
            });
        }

        // --- Draw callback ---
        let compositor_draw = compositor.clone();
        static DRAW_LOG_COUNT: AtomicU32 = AtomicU32::new(0);
        drawing_area.set_draw_func(move |_area, cr, w, h| {
            let log_this = DRAW_LOG_COUNT.fetch_add(1, Ordering::Relaxed) < 5;
            match compositor_draw.read().frame_for(tab_id) {
                None => {
                    if log_this { log::info!("[draw] frame_for={:?} → no frame yet (placeholder)", tab_id); }
                    draw_placeholder(cr, w, h);
                }
                Some(handle) => match handle {
                    ExternalHandle::CpuPixelsPtr {
                        width,
                        height,
                        stride,
                        pixel_buf,
                    } => {
                        if log_this {
                            log::info!("[draw] CpuPixelsPtr {}×{} stride={} area={}×{}", width, height, stride, w, h);
                            // SAFETY: pixel_buf is valid (same unsafe block below verifies this).
                            let p = unsafe { std::slice::from_raw_parts(pixel_buf.as_ptr(), 4.min((height as usize) * (stride as usize))) };
                            if p.len() >= 4 {
                                log::info!("[draw]   pixel[0] = {:02x} {:02x} {:02x} {:02x} (BGRA)", p[0], p[1], p[2], p[3]);
                            }
                        }
                        // SAFETY: pixel_buf is valid for the duration of this draw call.
                        let surface = unsafe {
                            gtk4::cairo::ImageSurface::create_for_data_unsafe(
                                pixel_buf.as_ptr(),
                                gtk4::cairo::Format::ARgb32,
                                width as i32,
                                height as i32,
                                stride as i32,
                            )
                            .expect("cairo surface (ptr)")
                        };
                        surface.flush();
                        scale_to_fit(cr, width as f64, height as f64, w, h);
                        cr.set_source_surface(&surface, 0.0, 0.0).unwrap_or_default();
                        cr.paint().unwrap_or_default();
                    }
                    ExternalHandle::CpuPixelsOwned {
                        width,
                        height,
                        stride,
                        mut pixels,
                        ..
                    } => {
                        if log_this {
                            log::info!("[draw] CpuPixelsOwned {}×{} stride={} area={}×{}", width, height, stride, w, h);
                            if pixels.len() >= 4 {
                                log::info!("[draw]   pixel[0,0]   = {:02x} {:02x} {:02x} {:02x} (BGRA)", pixels[0], pixels[1], pixels[2], pixels[3]);
                            }
                            // Sample a few pixels across the content area
                            for (px, py) in [(200u32, 100u32), (550, 50), (550, 100), (400, 150)] {
                                let off = (py as usize * stride as usize) + (px as usize * 4);
                                if pixels.len() > off + 3 {
                                    log::info!("[draw]   pixel[{},{}] = {:02x} {:02x} {:02x} {:02x} (BGRA)", px, py, pixels[off], pixels[off+1], pixels[off+2], pixels[off+3]);
                                }
                            }
                        }
                        // SAFETY: pixels is owned and valid for the duration of this draw call.
                        let surface = unsafe {
                            gtk4::cairo::ImageSurface::create_for_data_unsafe(
                                pixels.as_mut_ptr(),
                                gtk4::cairo::Format::ARgb32,
                                width as i32,
                                height as i32,
                                stride as i32,
                            )
                            .expect("cairo surface (owned)")
                        };
                        surface.flush();
                        scale_to_fit(cr, width as f64, height as f64, w, h);
                        cr.set_source_surface(&surface, 0.0, 0.0).unwrap_or_default();
                        cr.paint().unwrap_or_default();
                    }
                    other => {
                        if log_this { log::info!("[draw] unexpected handle variant: {:?}", other); }
                        draw_placeholder(cr, w, h);
                    }
                },
            }
        });

        // Resize → notify the engine tab
        drawing_area.connect_resize({
            let tab = tab.clone();
            move |_area, w, h| {
                let tab = tab.borrow().clone();
                TOKIO_RT.spawn(async move {
                    let _ = tab
                        .send(TabCommand::SetViewport {
                            x: 0,
                            y: 0,
                            width: w as u32,
                            height: h as u32,
                        })
                        .await;
                });
            }
        });

        // Address bar: navigate on Enter
        address_entry.connect_activate({
            let tab = tab.clone();
            let da = drawing_area.clone();
            move |entry| {
                let mut s = entry.text().to_string();
                if !s.starts_with("http://") && !s.starts_with("https://") {
                    s = format!("https://{s}");
                    entry.set_text(&s);
                }
                let Ok(url) = Url::parse(&s) else { return };
                let tab = tab.borrow().clone();
                let url_str = url.to_string();
                TOKIO_RT.spawn(async move {
                    let _ = tab.send(TabCommand::Navigate { url: url_str }).await;
                    let _ = tab.send(TabCommand::ResumeDrawing { fps: 30 }).await;
                });
                da.queue_draw();
            }
        });

        // Engine events → UI thread
        let (ui_tx, mut ui_rx) = mpsc::unbounded_channel::<EngineEvent>();
        TOKIO_RT.spawn({
            let ui_tx = ui_tx.clone();
            let mut rx = event_rx;
            async move {
                while let Ok(evt) = rx.recv().await {
                    let _ = ui_tx.send(evt);
                }
            }
        });
        {
            let da = drawing_area.clone();
            glib::spawn_future_local(async move {
                while let Some(evt) = ui_rx.recv().await {
                    match evt {
                        EngineEvent::Redraw { .. } => da.queue_draw(),
                        EngineEvent::Navigation { tab_id: _, event } => {
                            log::info!("navigation: {event:?}");
                        }
                        _ => {}
                    }
                }
            });
        }

        // Layout
        let url_bar = GtkBox::new(Orientation::Horizontal, 0);
        url_bar.append(&address_entry);

        let vbox = GtkBox::new(Orientation::Vertical, 0);
        vbox.append(&url_bar);
        vbox.append(&drawing_area);

        let window = ApplicationWindow::builder()
            .application(app)
            .title("Gosub Pipeline Browser")
            .default_width(1024)
            .default_height(800)
            .child(&vbox)
            .build();

        window.present();
    });

    // Pass only argv[0] to GTK so the URL argument isn't treated as a filename.
    let argv0: Vec<String> = std::env::args().take(1).collect();
    app.run_with_args(&argv0);
}

/// Apply a uniform scale so the frame fits the GTK drawing area.
fn scale_to_fit(cr: &gtk4::cairo::Context, frame_w: f64, frame_h: f64, area_w: i32, area_h: i32) {
    if frame_w > 0.0 && frame_h > 0.0 && (frame_w as i32 != area_w || frame_h as i32 != area_h) {
        cr.scale(area_w as f64 / frame_w, area_h as f64 / frame_h);
    }
}

/// Draw a light-grey placeholder while the first frame hasn't arrived yet.
fn draw_placeholder(cr: &gtk4::cairo::Context, w: i32, h: i32) {
    cr.set_source_rgb(0.92, 0.92, 0.92);
    cr.rectangle(0.0, 0.0, w as f64, h as f64);
    cr.fill().unwrap_or_default();
}
