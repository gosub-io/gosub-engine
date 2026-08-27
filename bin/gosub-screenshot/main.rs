//! Headless screenshot tool: loads a URL through the full gosub render pipeline and
//! saves the result as a PNG without opening a window.
//!
//! Uses the Skia backend for **CPU** rasterization - no GPU, no wgpu adapter, and no
//! system libraries (skia-safe is statically linked). The page is rasterized into small
//! cached tiles (`ExternalHandle::TileCache`) which we composite here, so there is no
//! GPU texture-size limit and pages of any height can be captured.

use clap::Parser;
use gosub_engine::events::{EngineEvent, Modifiers, MouseButton, NavigationEvent, TabCommand};
use gosub_engine::storage::{InMemorySessionStore, PartitionPolicy, SqliteLocalStore, StorageService};
use gosub_engine::tab::{TabDefaults, TabId};
use gosub_engine::zone::{ZoneConfig, ZoneId, ZoneServices};
use gosub_engine::DefaultRenderConfig;
use gosub_engine::GosubEngine;
use gosub_render_pipeline::render::backend::ExternalHandle;
use gosub_render_pipeline::render::DefaultCompositor;
#[cfg(all(feature = "backend_skia", not(feature = "backend_cairo")))]
use gosub_renderer_skia::{SkiaBackend, SkiaFontSystem};
use image::ColorType;

#[cfg(feature = "backend_cairo")]
use gosub_renderer_cairo::{CairoBackend, PangoFontSystem};
use once_cell::sync::Lazy;
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
#[cfg(all(feature = "backend_skia", not(feature = "backend_cairo")))]
type AppConfig = DefaultRenderConfig<SkiaBackend, SkiaFontSystem>;

/// CPU-only render configuration: Cairo rasterizer + Pango font system, no GPU/GTK window.
#[cfg(feature = "backend_cairo")]
type AppConfig = DefaultRenderConfig<CairoBackend, PangoFontSystem>;

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
    /// Extra seconds to wait after the first render, so async media (images)
    /// decode and repaint before the capture
    #[arg(long, default_value = "0")]
    settle: u64,
    /// Interactions to replay after the first render, before capturing: `click:X,Y`,
    /// `dblclick:X,Y`, `move:X,Y`, `press:X,Y` / `release:X,Y` (drags; page px), `tab`,
    /// `shift-tab`, `key:NAME` (e.g. `key:Backspace`, `key:ctrl+shift+ArrowLeft`), `type:TEXT`,
    /// `scroll:DX,DY` (wheel) or `wait:MS`. Repeatable. Ctrl+C/X/V use a clipboard local to
    /// this run.
    #[arg(short = 'i', long = "interact")]
    interact: Vec<String>,
    /// Print the engine's timing table on exit
    #[arg(long)]
    timings: bool,
    /// Render with the dark colour scheme (prefers-color-scheme: dark, dark native controls)
    #[arg(long)]
    dark: bool,
}

#[derive(Clone)]
enum Step {
    Send(TabCommand),
    Wait(Duration),
}

fn parse_interaction(spec: &str) -> Vec<Step> {
    let key = |k: &str, modifiers: Modifiers| {
        Step::Send(TabCommand::KeyDown {
            key: k.to_string(),
            code: k.to_string(),
            modifiers,
        })
    };
    match spec.split_once(':') {
        None if spec.eq_ignore_ascii_case("tab") => vec![key("Tab", Modifiers::empty())],
        None if spec.eq_ignore_ascii_case("shift-tab") => vec![key("Tab", Modifiers::SHIFT)],
        Some((kind, rest)) if kind.eq_ignore_ascii_case("key") => {
            // `ctrl+shift+ArrowLeft`: modifier prefixes, the key name last.
            let mut modifiers = Modifiers::empty();
            let mut name = rest;
            while let Some((m, tail)) = name.split_once('+').filter(|(_, t)| !t.is_empty()) {
                modifiers |= if m.eq_ignore_ascii_case("ctrl") || m.eq_ignore_ascii_case("control") {
                    Modifiers::CONTROL
                } else if m.eq_ignore_ascii_case("shift") {
                    Modifiers::SHIFT
                } else if m.eq_ignore_ascii_case("alt") {
                    Modifiers::ALT
                } else if ["meta", "cmd", "super"].iter().any(|k| m.eq_ignore_ascii_case(k)) {
                    Modifiers::META
                } else {
                    // Don't fold it into the key name - the engine would silently ignore the
                    // resulting `KeyDown` and the replay would do nothing.
                    eprintln!("Unknown modifier '{m}' in '{spec}': expected ctrl | shift | alt | meta");
                    std::process::exit(2);
                };
                name = tail;
            }
            vec![key(name, modifiers)]
        }
        Some((kind, rest)) if kind.eq_ignore_ascii_case("type") => {
            rest.chars().map(|c| key(&c.to_string(), Modifiers::empty())).collect()
        }
        Some((kind, rest)) if kind.eq_ignore_ascii_case("scroll") => {
            let parsed = rest
                .split_once(',')
                .and_then(|(x, y)| Some((x.trim().parse::<f32>().ok()?, y.trim().parse::<f32>().ok()?)));
            let Some((delta_x, delta_y)) = parsed else {
                eprintln!("Bad scroll spec '{spec}': expected scroll:DX,DY");
                std::process::exit(2);
            };
            vec![Step::Send(TabCommand::MouseScroll { delta_x, delta_y })]
        }
        Some((kind, rest)) if kind.eq_ignore_ascii_case("wait") => match rest.trim().parse::<u64>() {
            Ok(ms) => vec![Step::Wait(Duration::from_millis(ms))],
            Err(_) => {
                eprintln!("Bad wait spec '{spec}': expected wait:MILLISECONDS");
                std::process::exit(2);
            }
        },
        Some((kind, rest))
            if ["click", "dblclick", "move", "press", "release"]
                .iter()
                .any(|k| kind.eq_ignore_ascii_case(k)) =>
        {
            let Some((x, y)) = rest.split_once(',') else {
                eprintln!("Bad spec '{spec}': expected {kind}:X,Y");
                std::process::exit(2);
            };
            let (Ok(x), Ok(y)) = (x.trim().parse::<f32>(), y.trim().parse::<f32>()) else {
                eprintln!("Bad coordinates in '{spec}'");
                std::process::exit(2);
            };
            let down = Step::Send(TabCommand::MouseDown {
                x,
                y,
                button: MouseButton::Left,
            });
            let up = Step::Send(TabCommand::MouseUp {
                x,
                y,
                button: MouseButton::Left,
            });
            // Hover first: link activation piggybacks on hover state.
            let mut steps = vec![Step::Send(TabCommand::MouseMove { x, y })];
            if kind.eq_ignore_ascii_case("click") {
                steps.extend([down, up]);
            } else if kind.eq_ignore_ascii_case("dblclick") {
                steps.extend([down.clone(), up.clone(), down, up]);
            } else if kind.eq_ignore_ascii_case("press") {
                steps.push(down);
            } else if kind.eq_ignore_ascii_case("release") {
                steps.push(up);
            }
            steps
        }
        _ => {
            eprintln!(
                "Unknown interaction '{spec}' (expected click:X,Y | dblclick:X,Y | move:X,Y | press:X,Y | release:X,Y | scroll:DX,DY | tab | shift-tab | key:NAME | type:TEXT | wait:MS)"
            );
            std::process::exit(2);
        }
    }
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

    eprintln!("gosub-screenshot {BUILD_VERSION} — headless full-page screenshot via the gosub render pipeline (CPU rasterization)");

    let url = Url::parse(&url_str).expect("invalid URL");

    // ── Engine setup (CPU backend - no GPU) ───────────────────────────────────
    #[cfg(all(feature = "backend_skia", not(feature = "backend_cairo")))]
    let backend = SkiaBackend::new();
    #[cfg(feature = "backend_cairo")]
    let backend = CairoBackend::new();

    let _rt_guard = TOKIO_RT.enter();

    // Redraw notifications: engine → main thread.
    let (tx_redraw, rx_redraw) = std::sync::mpsc::channel::<()>();

    let compositor = Arc::new(DefaultCompositor::new(move || {
        let _ = tx_redraw.send(());
    }));

    let mut engine = GosubEngine::<AppConfig>::new(None, Arc::new(backend), compositor.clone());
    if args.dark {
        let _ = engine.settings().set(
            "renderer.color_scheme",
            gosub_config::settings::Setting::String("dark".into()),
        );
    }
    let _engine_task = TOKIO_RT.spawn(engine.start().expect("engine start"));
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
        places: None,
    };

    let mut zone = engine
        .create_zone(Some(zone_cfg), zone_services, Some(ZoneId::from(DEFAULT_ZONE)))
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

    if args.settle > 0 {
        std::thread::sleep(Duration::from_secs(args.settle));
        while rx_redraw.try_recv().is_ok() {}
    }

    // ── Phase 1b: replay interactions, waiting for each repaint ──────────────
    if !args.interact.is_empty() {
        let steps: Vec<Step> = args.interact.iter().flat_map(|s| parse_interaction(s)).collect();
        eprintln!("Replaying {} interaction(s)…", args.interact.len());
        // Ctrl+C/X/V in the page go through the embedder; here that is a string.
        let mut clipboard = String::new();
        for step in steps {
            let cmd = match step {
                Step::Wait(d) => {
                    std::thread::sleep(d);
                    while rx_redraw.try_recv().is_ok() {}
                    continue;
                }
                Step::Send(cmd) => cmd,
            };
            // MouseUp is in here too: checkbox toggles, submits and drag completion commit on
            // release, so without the wait the capture can race ahead of the effect.
            let is_input = matches!(
                cmd,
                TabCommand::MouseDown { .. }
                    | TabCommand::MouseUp { .. }
                    | TabCommand::KeyDown { .. }
                    | TabCommand::MouseScroll { .. }
            );
            let is_move = matches!(cmd, TabCommand::MouseMove { .. });
            // Ctrl+C/X/V: wait for the clipboard event (not a possibly stale repaint) first.
            let clip_chord = match &cmd {
                TabCommand::KeyDown { key, modifiers, .. }
                    if modifiers.intersects(Modifiers::CONTROL | Modifiers::META) =>
                {
                    match key.as_str() {
                        "c" | "C" => Some("c"),
                        "x" | "X" => Some("x"),
                        "v" | "V" => Some("v"),
                        _ => None,
                    }
                }
                _ => None,
            };
            let tab_i = tab.clone();
            TOKIO_RT.block_on(async move {
                let _ = tab_i.send(cmd).await;
            });
            if is_move {
                // Swallow the hover repaint so it isn't mistaken for the click's.
                std::thread::sleep(Duration::from_millis(200));
                while rx_redraw.try_recv().is_ok() {}
                continue;
            }
            if !is_input {
                continue;
            }
            let deadline = Instant::now() + Duration::from_secs(5);
            let mut repainted = false;
            let mut clip_pending = clip_chord.is_some();
            while Instant::now() < deadline {
                while let Ok(ev) = event_rx.try_recv() {
                    match ev {
                        EngineEvent::ClipboardWrite { text, .. } => {
                            clipboard = text;
                            clip_pending = false;
                        }
                        EngineEvent::PasteRequested { .. } => {
                            let tab_i = tab.clone();
                            let text = clipboard.clone();
                            TOKIO_RT.block_on(async move {
                                let _ = tab_i.send(TabCommand::TextInput { text }).await;
                            });
                            clip_pending = false;
                        }
                        _ => {}
                    }
                }
                if clip_pending {
                    std::thread::sleep(Duration::from_millis(20));
                    continue;
                }
                // A copy changes nothing on screen; cut/paste repaint.
                if clip_chord == Some("c") {
                    repainted = true;
                    break;
                }
                if rx_redraw.try_recv().is_ok() {
                    repainted = true;
                    std::thread::sleep(Duration::from_millis(100));
                    while rx_redraw.try_recv().is_ok() {}
                    break;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            if !repainted {
                eprintln!("(no repaint observed after interaction - focus state may be unchanged)");
            }
        }
    }

    let phase1_handle = compositor.frame_for(tab_id);
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
            if let Some(ExternalHandle::TileCache { .. }) = compositor.frame_for(tab_id) {
                tile_cache_handle = compositor.frame_for(tab_id);
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
        // Group opacity of the tile's layer (e.g. a translucent fixed navbar). Pixels are
        // premultiplied, so scaling all four channels by it fades the whole tile as one group.
        let op = tile.opacity.clamp(0.0, 1.0);

        for row in 0..th {
            for col in 0..tw {
                let src_off = (row * tile.width as usize + col) * 4;
                let dst_off = ((ty as usize + row) * page_w as usize + (tx as usize + col)) * 4;

                let (r, g, b, a) = if op >= 1.0 {
                    (
                        data[src_off] as u32,
                        data[src_off + 1] as u32,
                        data[src_off + 2] as u32,
                        data[src_off + 3] as u32,
                    )
                } else {
                    (
                        (data[src_off] as f32 * op).round() as u32,
                        (data[src_off + 1] as f32 * op).round() as u32,
                        (data[src_off + 2] as f32 * op).round() as u32,
                        (data[src_off + 3] as f32 * op).round() as u32,
                    )
                };

                // Premultiplied source-over the *existing* buffer (initialised to white), not a
                // fixed white background: result = src_rgb + (255 - src_a)/255 * dst_rgb. Blending
                // over the buffer (rather than overwriting) lets an upper layer's transparent
                // regions reveal content from layers already composited beneath it - e.g. a
                // promoted `position: fixed` navbar tile must not erase the rows behind it.
                let inv_a = 255u32 - a;
                let (d0, d1, d2) = (
                    pixels[dst_off] as u32,
                    pixels[dst_off + 1] as u32,
                    pixels[dst_off + 2] as u32,
                );
                pixels[dst_off] = (r + d0 * inv_a / 255).min(255) as u8;
                pixels[dst_off + 1] = (g + d1 * inv_a / 255).min(255) as u8;
                pixels[dst_off + 2] = (b + d2 * inv_a / 255).min(255) as u8;
                // dst alpha stays 255 (opaque output)
            }
        }
    }

    image::save_buffer(&output, &pixels, page_w, page_h, ColorType::Rgba8).expect("save PNG");
    eprintln!("Saved {output} ({}×{})", page_w, page_h);
    if args.timings {
        gosub_shared::timing::dump(false);
    }
}
