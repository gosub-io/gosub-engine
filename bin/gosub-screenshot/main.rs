//! Headless screenshot tool: loads a URL through the full gosub_pipeline render system and
//! saves the result as a PNG without opening a window.
//!
//! No display server required — uses Cairo in software rendering mode.

use cairo::{Context, Format, ImageSurface};
use clap::Parser;
use gosub_engine::events::{EngineEvent, NavigationEvent, TabCommand};
use gosub_engine::storage::{InMemorySessionStore, PartitionPolicy, SqliteLocalStore, StorageService};
use gosub_engine::tab::{TabDefaults, TabId};
use gosub_engine::zone::{ZoneConfig, ZoneId, ZoneServices};
use gosub_engine::GosubEngine;
use gosub_render_pipeline::render::backend::ExternalHandle;
use gosub_render_pipeline::render::backends::cairo::CairoBackend;
use gosub_render_pipeline::render::DefaultCompositor;
use image::ColorType;
use once_cell::sync::Lazy;
use parking_lot::RwLock;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::runtime::{Builder, Runtime};
use url::Url;
use uuid::uuid;

const BUILD_VERSION: &str = concat!(
    env!("CARGO_PKG_VERSION"),
    " (",
    env!("BUILD_GIT_SHA"),
    " · ",
    env!("BUILD_DATE"),
    ")"
);

#[derive(Parser)]
#[command(name = "gosub-screenshot", version = BUILD_VERSION, about = "Headless screenshot tool using the GoSub render pipeline")]
struct Args {
    /// URL to capture (https:// is prepended if no scheme is given)
    url: String,
    /// Output PNG path
    #[arg(default_value = "screenshot.png")]
    output: String,
    /// Viewport width in CSS pixels
    #[arg(default_value = "1280")]
    width: u32,
    /// Seconds to wait for navigation to complete
    #[arg(long, default_value = "30")]
    nav_timeout: u64,
    /// Seconds to wait for the first render after navigation completes
    #[arg(long, default_value = "120")]
    render_timeout: u64,
}

const DEFAULT_ZONE: uuid::Uuid = uuid!("f1234567-abcd-4000-8000-000000000003");
/// Maximum page height to capture, in CSS pixels. Prevents out-of-memory on huge pages.
const MAX_PAGE_HEIGHT: u32 = 16384;

static TOKIO_RT: Lazy<Runtime> = Lazy::new(|| {
    Builder::new_multi_thread()
        .enable_io()
        .enable_time()
        .thread_name("gosub-screenshot-rt")
        .build()
        .expect("tokio runtime")
});

fn main() {
    simple_logger::SimpleLogger::new()
        .with_level(log::LevelFilter::Warn)
        .env()
        .init()
        .unwrap_or_default();

    let args = Args::parse();
    let url_str = if args.url.contains("://") {
        args.url.clone()
    } else {
        format!("https://{}", args.url)
    };
    let output = args.output;
    let viewport_w = args.width;

    eprintln!("gosub-screenshot {BUILD_VERSION}");

    let url = Url::parse(&url_str).expect("invalid URL");

    let _rt_guard = TOKIO_RT.enter();

    // Redraw notifications: engine → main thread.
    let (tx_redraw, rx_redraw) = std::sync::mpsc::channel::<()>();

    let compositor = Arc::new(RwLock::new(DefaultCompositor::new(move || {
        let _ = tx_redraw.send(());
    })));

    let backend = CairoBackend::new();
    let mut engine = GosubEngine::new(None, Arc::new(backend), compositor.clone());
    let _join = engine.start().expect("engine start");
    let mut event_rx = engine.subscribe_events();

    let zone_cfg = ZoneConfig::builder().build().expect("ZoneConfig");
    let zone_services = ZoneServices {
        storage: Arc::new(StorageService::new(
            Arc::new(SqliteLocalStore::new(":memory:").expect("local store")),
            Arc::new(InMemorySessionStore::new()),
        )),
        cookie_store: None,
        cookie_jar: None,
        partition_policy: PartitionPolicy::None,
    };

    let mut zone = engine
        .create_zone(zone_cfg, zone_services, Some(ZoneId::from(DEFAULT_ZONE)))
        .expect("create_zone");

    let tab = TOKIO_RT
        .block_on(zone.create_tab(
            TabDefaults {
                url: None,
                title: Some("screenshot".to_string()),
                viewport: None,
            },
            None,
        ))
        .expect("create_tab");

    let tab_id: TabId = tab.tab_id;

    // Use a tall initial viewport so the full page is laid out and rasterized.
    // After the first render we trigger a 1-pixel scroll to get a TileCache handle
    // which carries page_height — then we composite only the real content rows.
    let tab_nav = tab.clone();
    TOKIO_RT.spawn(async move {
        let _ = tab_nav
            .send(TabCommand::SetViewport {
                x: 0,
                y: 0,
                width: viewport_w,
                height: MAX_PAGE_HEIGHT,
            })
            .await;
        let _ = tab_nav.send(TabCommand::Navigate { url: url.to_string() }).await;
        let _ = tab_nav.send(TabCommand::ResumeDrawing { fps: 30 }).await;
    });

    let nav_deadline = Instant::now() + Duration::from_secs(args.nav_timeout);
    // Render deadline is set once navigation completes; its budget starts then.
    let render_budget = Duration::from_secs(args.render_timeout);
    let mut render_deadline: Option<Instant> = None;

    eprintln!("Loading {url_str} (viewport width={viewport_w})…");

    // ── Phase 1: wait for navigation to complete + first full render ──────────────
    // Two separate budgets: nav_timeout for downloading the page, render_timeout for
    // the CPU-bound pipeline (layout + rasterization). Complex pages can take tens of
    // seconds in the pipeline even after HTML arrives.
    let mut nav_done = false;
    let mut first_render_done = false;

    loop {
        let now = Instant::now();
        if !nav_done && now >= nav_deadline {
            eprintln!("Timeout waiting for navigation ({}s)", args.nav_timeout);
            std::process::exit(1);
        }
        if let Some(rd) = render_deadline {
            if now >= rd {
                eprintln!("Timeout waiting for first render ({}s)", args.render_timeout);
                std::process::exit(1);
            }
        }

        while rx_redraw.try_recv().is_ok() {
            if nav_done {
                first_render_done = true;
            }
        }

        loop {
            match event_rx.try_recv() {
                Ok(EngineEvent::Navigation { tab_id: tid, event }) if tid == tab_id => match event {
                    NavigationEvent::Finished { .. } => {
                        eprintln!("Navigation finished.");
                        nav_done = true;
                        render_deadline = Some(Instant::now() + render_budget);
                    }
                    NavigationEvent::Failed { error, .. } => {
                        eprintln!("Navigation failed: {error}");
                        std::process::exit(1);
                    }
                    NavigationEvent::FailedUrl { error, .. } => {
                        eprintln!("Invalid URL: {error}");
                        std::process::exit(1);
                    }
                    _ => {}
                },
                Ok(_) => {}
                Err(_) => break,
            }
        }

        if nav_done && first_render_done {
            break;
        }

        std::thread::sleep(Duration::from_millis(50));
    }

    // Seed tile_cache_handle / cpu_frame from the Phase 1 frame. The rasterizer may
    // already have submitted a TileCache (e.g. gosub_renderer_cairo), so we don't
    // need Phase 2 to produce one from scratch. Phase 2 can still upgrade it with
    // a post-scroll frame that carries the final page_height.
    let phase1_frame = compositor.read().frame_for(tab_id);
    let (mut tile_cache_handle, mut cpu_frame): (Option<ExternalHandle>, Option<ExternalHandle>) =
        match phase1_frame {
            Some(h @ ExternalHandle::TileCache { .. }) => (Some(h), None),
            Some(h @ ExternalHandle::CpuPixelsOwned { .. }) => (None, Some(h)),
            _ => (None, None),
        };

    // ── Phase 2: trigger a 1px scroll to get TileCache which carries page_height ──
    // The pipeline already has cached tiles; the scroll just swaps the handle type.
    let tab_scroll = tab.clone();
    TOKIO_RT.spawn(async move {
        let _ = tab_scroll
            .send(TabCommand::MouseScroll {
                delta_x: 0.0,
                delta_y: 1.0,
            })
            .await;
    });

    let deadline2 = Instant::now() + Duration::from_secs(5);

    while Instant::now() < deadline2 {
        while rx_redraw.try_recv().is_ok() {
            if let Some(handle) = compositor.read().frame_for(tab_id) {
                match &handle {
                    ExternalHandle::TileCache { .. } => tile_cache_handle = Some(handle),
                    ExternalHandle::CpuPixelsOwned { .. } => cpu_frame = Some(handle),
                    _ => {}
                }
            }
        }
        if tile_cache_handle.is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }

    // ── Phase 3: composite tiles into a full-page PNG ────────────────────────────
    let (tiles, dpr, page_height_f) = match tile_cache_handle {
        Some(ExternalHandle::TileCache {
            tiles,
            dpr,
            page_height,
            ..
        }) => (tiles, dpr, page_height),
        _ => {
            // TileCache unavailable (e.g. Cairo backend, or page too short to scroll).
            // Use the best CPU-pixels frame captured during Phase 1/2.
            eprintln!("TileCache not available; falling back to composited frame.");
            let handle = cpu_frame.unwrap_or_else(|| {
                eprintln!("No rendered frame available.");
                std::process::exit(1);
            });
            match handle {
                ExternalHandle::CpuPixelsOwned {
                    width,
                    height,
                    stride,
                    pixels,
                    ..
                } => {
                    let actual_h = crop_height(&pixels, width, height, stride as usize);
                    eprintln!("Saving {}x{} (cropped from {})", width, actual_h, height);
                    save_argb32_as_png(&pixels, width, actual_h, stride as usize, &output);
                    return;
                }
                _ => {
                    eprintln!("Unexpected handle type");
                    std::process::exit(1);
                }
            }
        }
    };

    let page_h = (page_height_f.ceil() as u32).clamp(1, MAX_PAGE_HEIGHT);
    let dpr_f = dpr as f64;

    eprintln!(
        "Page size: {}×{} CSS px (DPR {}). Compositing {} tile(s)…",
        viewport_w,
        page_h,
        dpr,
        tiles.len()
    );

    // Composite all tiles at their page-coordinate positions onto a surface covering
    // the full page (scroll offset = 0).
    let surface = ImageSurface::create(Format::ARgb32, viewport_w as i32, page_h as i32).expect("ImageSurface create");
    let cr = Context::new(&surface).expect("Cairo context");

    cr.set_source_rgb(1.0, 1.0, 1.0);
    cr.rectangle(0.0, 0.0, viewport_w as f64, page_h as f64);
    cr.fill().unwrap_or_default();

    for tile in tiles.iter() {
        // SAFETY: tile.data is Arc-owned and lives for this compositing call.
        #[allow(unsafe_code)]
        let tile_surface = unsafe {
            ImageSurface::create_for_data_unsafe(
                tile.data.as_ptr() as *mut u8,
                Format::ARgb32,
                tile.width as i32,
                tile.height as i32,
                (tile.width * 4) as i32,
            )
        };
        if let Ok(ts) = tile_surface {
            ts.set_device_scale(dpr_f, dpr_f);
            cr.set_source_surface(&ts, tile.page_x as f64, tile.page_y as f64)
                .unwrap_or_default();
            cr.paint().unwrap_or_default();
        }
    }
    drop(cr);
    surface.flush();

    let mut surface = surface;
    let stride = surface.stride() as usize;
    let raw = surface.data().expect("surface data");
    save_argb32_as_png(&raw, viewport_w, page_h, stride, &output);
}

/// Detect the last row that contains non-white pixels (for fallback cropping).
fn crop_height(pixels: &[u8], width: u32, height: u32, stride: usize) -> u32 {
    for row in (0..height as usize).rev() {
        for col in 0..width as usize {
            let off = row * stride + col * 4;
            let b = pixels[off];
            let g = pixels[off + 1];
            let r = pixels[off + 2];
            if r != 255 || g != 255 || b != 255 {
                return (row + 1) as u32;
            }
        }
    }
    height
}

/// Convert ARgb32 (premultiplied, LE: B G R A bytes) → RGBA8 straight-alpha and save as PNG.
fn save_argb32_as_png(pixels: &[u8], width: u32, height: u32, stride: usize, path: &str) {
    let mut rgba: Vec<u8> = Vec::with_capacity((width * height * 4) as usize);
    for row in 0..height as usize {
        for col in 0..width as usize {
            let off = row * stride + col * 4;
            let b = pixels[off];
            let g = pixels[off + 1];
            let r = pixels[off + 2];
            let a = pixels[off + 3];
            if a == 0 {
                rgba.extend_from_slice(&[0, 0, 0, 0]);
            } else if a == 255 {
                rgba.extend_from_slice(&[r, g, b, 255]);
            } else {
                let af = a as f32 / 255.0;
                rgba.push((r as f32 / af).min(255.0).round() as u8);
                rgba.push((g as f32 / af).min(255.0).round() as u8);
                rgba.push((b as f32 / af).min(255.0).round() as u8);
                rgba.push(a);
            }
        }
    }
    image::save_buffer(path, &rgba, width, height, ColorType::Rgba8).expect("save PNG");
    eprintln!("Saved {path} ({}×{})", width, height);
}
