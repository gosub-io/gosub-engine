//! Headless screenshot tool: loads a URL through the full gosub render pipeline and
//! saves the result as a PNG without opening a window.
//!
//! Uses the Skia backend for **CPU** rasterization — no GPU, no wgpu adapter, and no
//! system libraries (skia-safe is statically linked). The page is rasterized into small
//! cached tiles (`ExternalHandle::TileCache`) which we composite here, so there is no
//! GPU texture-size limit and pages of any height can be captured.

use clap::Parser;
use gosub_engine::events::{EngineEvent, NavigationEvent, TabCommand};
use gosub_engine::storage::{InMemorySessionStore, PartitionPolicy, SqliteLocalStore, StorageService};
use gosub_engine::tab::{TabDefaults, TabId};
use gosub_engine::zone::{ZoneConfig, ZoneId, ZoneServices};
use gosub_engine::DefaultRenderConfig;
use gosub_engine::GosubEngine;
use gosub_render_pipeline::render::backend::ExternalHandle;
use gosub_render_pipeline::render::DefaultCompositor;
use gosub_renderer_skia::{SkiaBackend, SkiaFontSystem};
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

/// CPU-only render configuration: Skia rasterizer + Skia font system, no GPU.
type AppConfig = DefaultRenderConfig<SkiaBackend, SkiaFontSystem>;

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
/// Initial viewport height used for layout, in CSS pixels. Tall enough to trigger
/// below-the-fold / lazily-loaded content; the captured image uses the page's *true*
/// height, not this value. CPU rasterization has no GPU texture limit, so there is no
/// cap on how tall the final screenshot can be.
const INITIAL_VIEWPORT_HEIGHT: u32 = 16384;

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

    // ── Engine setup (CPU Skia backend — no GPU) ──────────────────────────────
    let backend = SkiaBackend::new();

    let _rt_guard = TOKIO_RT.enter();

    // Redraw notifications: engine → main thread.
    let (tx_redraw, rx_redraw) = std::sync::mpsc::channel::<()>();

    let compositor = Arc::new(RwLock::new(DefaultCompositor::new(move || {
        let _ = tx_redraw.send(());
    })));

    let mut engine = GosubEngine::<AppConfig>::new(None, Arc::new(backend), compositor.clone());
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
    let tab_nav = tab.clone();
    TOKIO_RT.spawn(async move {
        let _ = tab_nav
            .send(TabCommand::SetViewport {
                x: 0,
                y: 0,
                width: viewport_w,
                height: INITIAL_VIEWPORT_HEIGHT,
            })
            .await;
        let _ = tab_nav.send(TabCommand::Navigate { url: url.to_string() }).await;
        let _ = tab_nav.send(TabCommand::ResumeDrawing { fps: 30 }).await;
    });

    let nav_deadline = Instant::now() + Duration::from_secs(args.nav_timeout);
    let render_budget = Duration::from_secs(args.render_timeout);
    let mut render_deadline: Option<Instant> = None;
    let mut nav_done = false;
    let mut first_render_done = false;

    eprintln!("Loading {url_str} (viewport width={viewport_w})…");

    // ── Phase 1: wait for navigation + first full render ─────────────────────
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

    let phase1_handle = compositor.read().frame_for(tab_id);
    let mut tile_cache_handle: Option<ExternalHandle> = match phase1_handle {
        Some(h @ ExternalHandle::TileCache { .. }) => Some(h),
        _ => None,
    };

    // ── Phase 2: trigger a 1px scroll to obtain TileCache with page_height ───
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
            if let Some(ExternalHandle::TileCache { .. }) = compositor.read().frame_for(tab_id) {
                tile_cache_handle = compositor.read().frame_for(tab_id);
            }
        }
        if tile_cache_handle.is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }

    // ── Phase 3: composite tiles into a full-page PNG ────────────────────────
    let (tiles, page_height_f) = match tile_cache_handle {
        Some(ExternalHandle::TileCache { tiles, page_height, .. }) => (tiles, page_height),
        _ => {
            eprintln!("No TileCache frame available — nothing was rendered.");
            std::process::exit(1);
        }
    };

    let page_w = viewport_w;
    let page_h = (page_height_f.ceil() as u32).max(1);

    eprintln!(
        "Page size: {}×{} px. Compositing {} tile(s)…",
        page_w,
        page_h,
        tiles.len()
    );

    // Fill with opaque white, then alpha-blend each tile (premultiplied).
    let mut pixels = vec![255u8; (page_w * page_h * 4) as usize];

    for tile in tiles.iter() {
        let tx = tile.page_x as u32;
        let ty = tile.page_y as u32;
        if tx >= page_w || ty >= page_h {
            continue;
        }
        let tw = tile.width.min(page_w - tx) as usize;
        let th = tile.height.min(page_h - ty) as usize;
        // Normalize to [R, G, B, A] regardless of which rasterizer produced the tile.
        // Skia produces premultiplied ARGB32 ([B, G, R, A]); `to_rgba` swaps as needed.
        let data = tile.format.to_rgba(&tile.data);

        for row in 0..th {
            for col in 0..tw {
                let src_off = (row * tile.width as usize + col) * 4;
                let dst_off = ((ty as usize + row) * page_w as usize + (tx as usize + col)) * 4;

                let r = data[src_off];
                let g = data[src_off + 1];
                let b = data[src_off + 2];
                let a = data[src_off + 3];

                // Premultiplied source-over the *existing* buffer (initialised to white), not a
                // fixed white background: result = src_rgb + (255 - src_a)/255 * dst_rgb. Blending
                // over the buffer (rather than overwriting) lets an upper layer's transparent
                // regions reveal content from layers already composited beneath it — e.g. a
                // promoted `position: fixed` navbar tile must not erase the rows behind it.
                let inv_a = 255u32 - a as u32;
                let (d0, d1, d2) = (
                    pixels[dst_off] as u32,
                    pixels[dst_off + 1] as u32,
                    pixels[dst_off + 2] as u32,
                );
                pixels[dst_off] = (r as u32 + d0 * inv_a / 255).min(255) as u8;
                pixels[dst_off + 1] = (g as u32 + d1 * inv_a / 255).min(255) as u8;
                pixels[dst_off + 2] = (b as u32 + d2 * inv_a / 255).min(255) as u8;
                // dst alpha stays 255 (opaque output)
            }
        }
    }

    image::save_buffer(&output, &pixels, page_w, page_h, ColorType::Rgba8).expect("save PNG");
    eprintln!("Saved {output} ({}×{})", page_w, page_h);
}
