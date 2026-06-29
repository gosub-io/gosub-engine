//! Minimal browser window: Skia (CPU) rasterizer + GTK4 toolkit.
//!
//! Usage: cargo run --example gtk4-skia -- https://example.com
//!
//! GTK4 is used only for windowing; Skia handles all rasterization and fonts.
//! No gtk4::init() needed for fonts — unlike the Cairo backend, Skia is self-contained.

use gosub_engine::cookies::SqliteCookieStore;
use gosub_engine::events::{EngineEvent, NavigationEvent, TabCommand};
use gosub_engine::storage::{InMemorySessionStore, PartitionPolicy, SqliteLocalStore, StorageService};
use gosub_engine::tab::{TabDefaults, TabId};
use gosub_engine::zone::{ZoneConfig, ZoneId, ZoneServices};
use gosub_engine::DefaultRenderConfig;
use gosub_engine::GosubEngine;
use gosub_render_pipeline::render::backend::{
    anchored_tile_pos, blend_over_argb_u32, scale_premul_argb_u32, CachedTile, ExternalHandle,
};
use gosub_render_pipeline::render::DefaultCompositor;
use gosub_renderer_skia::SkiaFontSystem;
use gtk4::glib;
use gtk4::prelude::*;
use gtk4::{Application, ApplicationWindow, Box as GtkBox, DrawingArea, Entry, Label, Orientation};
use once_cell::sync::Lazy;
use parking_lot::RwLock;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;
use tokio::runtime::{Builder, Runtime};
use tokio::sync::mpsc;
use url::Url;
use uuid::uuid;

const DEFAULT_ZONE: uuid::Uuid = uuid!("f1234567-abcd-4000-8000-00000000000b");
/// CSS pixels scrolled per raw GTK scroll unit.  Lower = more dampened.
const SCROLL_MULTIPLIER: f32 = 134.0;

type AppConfig = DefaultRenderConfig<gosub_renderer_skia::SkiaBackend, SkiaFontSystem>;

static TOKIO_RT: Lazy<Runtime> = Lazy::new(|| {
    Builder::new_multi_thread()
        .enable_io()
        .enable_time()
        .thread_name("gosub-pipeline-rt")
        .build()
        .expect("tokio runtime")
});

/// Cached tile render state extracted from the latest engine TileCache frame.
/// Lives on the GTK main thread — no locking needed.
struct TileDrawState {
    tiles: Arc<Vec<CachedTile>>,
    dpr: u32,
    viewport_width: u32,
    viewport_height: u32,
    page_height: f32,
}

fn main() {
    simple_logger::SimpleLogger::new()
        .with_level(log::LevelFilter::Warn)
        .env()
        .init()
        .unwrap_or_default();

    let initial_url: String = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "https://stop-ai-slop.com".to_string());

    let app = Application::builder().application_id("io.gosub.gtk4-skia").build();

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

        let backend = gosub_renderer_skia::SkiaBackend::new();
        let mut engine = GosubEngine::<AppConfig>::new(None, Arc::new(backend), compositor.clone());
        let _join = engine.start().expect("engine start");
        let event_rx = engine.subscribe_events();

        let zone_cfg = ZoneConfig::builder().do_not_track(true).build().expect("ZoneConfig");

        let cookie_store: gosub_engine::cookies::CookieStoreHandle =
            SqliteCookieStore::new(".pipeline-browser-cookies.db".into())
                .expect("failed to open cookie store")
                .into();

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
                        title: Some("Gosub Skia".to_string()),
                        // No initial viewport — let connect_resize set it with the correct DPR.
                        // If we pre-set a viewport here (DPR=1), the engine won't recreate the
                        // surface when connect_resize sends the same CSS dimensions with DPR=2.
                        viewport: None,
                    },
                    None,
                ))
                .expect("create_tab")
        };

        let tab_id: TabId = tab.tab_id;

        // Wrap the tab in Rc<RefCell<>> so closures can share it
        let tab = Rc::new(RefCell::new(tab));

        // --- Local tile/scroll state (GTK main thread only, no locking) ---
        //
        // When the engine completes a full render it sends a TileCache frame via the
        // compositor.  We extract the tiles + metadata here so the draw callback and
        // scroll handler can use them without any async roundtrip.
        let local_tiles: Rc<RefCell<Option<TileDrawState>>> = Rc::new(RefCell::new(None));
        // Current scroll offset in CSS px — updated synchronously in the GTK scroll
        // handler so every frame sees the very latest position without async latency.
        let local_scroll: Rc<Cell<(f32, f32)>> = Rc::new(Cell::new((0.0, 0.0)));
        // Handle for the active kinetic-scroll glib timeout (if any).
        let kinetic_source: Rc<RefCell<Option<glib::SourceId>>> = Rc::new(RefCell::new(None));

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

        // When the engine submits a new frame, check if it is a TileCache and stash it
        // in local_tiles so the draw callback can use it immediately.  Also sync the
        // local scroll position so kinetic deceleration stays consistent.
        {
            let da = drawing_area.clone();
            let compositor_rx = compositor.clone();
            let local_tiles = local_tiles.clone();
            let local_scroll = local_scroll.clone();
            glib::spawn_future_local(async move {
                while let Some(()) = rx_redraw.recv().await {
                    if let Some(ExternalHandle::TileCache {
                        tiles,
                        dpr,
                        viewport_width,
                        viewport_height,
                        page_height,
                        scroll_x,
                        scroll_y,
                    }) = compositor_rx.read().frame_for(tab_id)
                    {
                        *local_tiles.borrow_mut() = Some(TileDrawState {
                            tiles,
                            dpr,
                            viewport_width,
                            viewport_height,
                            page_height,
                        });
                        // Sync scroll — the engine may have clamped it.
                        local_scroll.set((scroll_x, scroll_y));
                    }
                    da.queue_draw();
                }
            });
        }

        // --- Draw callback ---
        //
        // Priority order:
        //   1. Local TileCache (zero-copy, uses up-to-date local scroll offset)
        //   2. Compositor frame (CpuPixelsOwned / CpuPixelsPtr — initial page render fallback)
        let compositor_draw = compositor.clone();
        let local_tiles_draw = local_tiles.clone();
        let local_scroll_draw = local_scroll.clone();
        drawing_area.set_draw_func(move |_area, cr, w, h| {
            // Fast path: use cached tiles with local scroll position.
            let tiles_opt = local_tiles_draw.borrow();
            if let Some(state) = tiles_opt.as_ref() {
                let (scroll_x, scroll_y) = local_scroll_draw.get();
                log::debug!(
                    "[draw] TileCache {}x{} dpr={} scroll=({:.1},{:.1}) tiles={}",
                    state.viewport_width,
                    state.viewport_height,
                    state.dpr,
                    scroll_x,
                    scroll_y,
                    state.tiles.len()
                );
                draw_tile_cache(cr, w, h, state, scroll_x, scroll_y);
                return;
            }
            drop(tiles_opt);

            // Slow path: engine hasn't produced a TileCache yet — use the compositor frame.
            match compositor_draw.read().frame_for(tab_id) {
                None => {
                    log::debug!("[draw] no frame yet — placeholder");
                    draw_placeholder(cr, w, h);
                }
                Some(handle) => match handle {
                    ExternalHandle::CpuPixelsPtr {
                        width,
                        height,
                        stride,
                        pixel_buf,
                    } => {
                        log::debug!(
                            "[draw] CpuPixelsPtr {}x{} stride={} widget={}x{}",
                            width,
                            height,
                            stride,
                            w,
                            h
                        );
                        let frame_scale = (width as f64 / w as f64).round() as i32;
                        let owned = unsafe {
                            std::slice::from_raw_parts(pixel_buf.as_ptr(), (height as usize) * (stride as usize))
                        }
                        .to_vec();
                        match gtk4::cairo::ImageSurface::create_for_data(
                            owned,
                            gtk4::cairo::Format::ARgb32,
                            width as i32,
                            height as i32,
                            stride as i32,
                        ) {
                            Ok(surface) => {
                                surface.flush();
                                if frame_scale > 1 {
                                    surface.set_device_scale(frame_scale as f64, frame_scale as f64);
                                }
                                cr.set_source_surface(&surface, 0.0, 0.0).unwrap_or_default();
                                cr.paint().unwrap_or_default();
                            }
                            Err(e) => {
                                log::warn!("[draw] surface failed: {:?}", e);
                                draw_placeholder(cr, w, h);
                            }
                        }
                    }
                    ExternalHandle::CpuPixelsOwned {
                        width,
                        height,
                        stride,
                        pixels,
                        ..
                    } => {
                        log::debug!(
                            "[draw] CpuPixelsOwned {}x{} stride={} bytes={} widget={}x{}",
                            width,
                            height,
                            stride,
                            pixels.len(),
                            w,
                            h
                        );
                        let frame_scale = (width as f64 / w as f64).round() as i32;
                        match gtk4::cairo::ImageSurface::create_for_data(
                            pixels,
                            gtk4::cairo::Format::ARgb32,
                            width as i32,
                            height as i32,
                            stride as i32,
                        ) {
                            Ok(surface) => {
                                surface.flush();
                                if frame_scale > 1 {
                                    surface.set_device_scale(frame_scale as f64, frame_scale as f64);
                                }
                                cr.set_source_surface(&surface, 0.0, 0.0).unwrap_or_default();
                                cr.paint().unwrap_or_default();
                            }
                            Err(e) => {
                                log::warn!("[draw] surface failed: {:?}", e);
                                draw_placeholder(cr, w, h);
                            }
                        }
                    }
                    ExternalHandle::TileCache {
                        viewport_width,
                        viewport_height,
                        dpr,
                        scroll_x,
                        scroll_y,
                        page_height,
                        tiles,
                    } => {
                        // This arm is only reached if local_tiles was empty — should be rare.
                        let state = TileDrawState {
                            tiles,
                            dpr,
                            viewport_width,
                            viewport_height,
                            page_height,
                        };
                        draw_tile_cache(cr, w, h, &state, scroll_x, scroll_y);
                    }
                    _ => {
                        log::debug!("[draw] NullHandle or other — placeholder");
                        draw_placeholder(cr, w, h);
                    }
                },
            }
        });

        // --- Scroll controller ---
        //
        // The local scroll offset is updated synchronously here (on the GTK main thread),
        // so queue_draw() immediately sees the new position — zero async latency.
        // The engine is also notified via a Tokio task for its own state bookkeeping.
        let scroll_ctl = gtk4::EventControllerScroll::new(
            gtk4::EventControllerScrollFlags::BOTH_AXES | gtk4::EventControllerScrollFlags::KINETIC,
        );

        // Cancel any in-progress kinetic scroll when a new gesture starts.
        scroll_ctl.connect_scroll_begin({
            let kinetic_source = kinetic_source.clone();
            move |_| {
                if let Some(id) = kinetic_source.borrow_mut().take() {
                    id.remove();
                }
            }
        });

        scroll_ctl.connect_scroll({
            let tab = tab.clone();
            let local_tiles = local_tiles.clone();
            let local_scroll = local_scroll.clone();
            let da = drawing_area.clone();
            move |_ctl, dx, dy| {
                let delta_x = dx as f32 * SCROLL_MULTIPLIER;
                let delta_y = dy as f32 * SCROLL_MULTIPLIER;

                // Update local scroll immediately (synchronous — no async roundtrip).
                let (prev_x, prev_y) = local_scroll.get();
                let max_y = local_tiles
                    .borrow()
                    .as_ref()
                    .map(|s| (s.page_height - s.viewport_height as f32).max(0.0))
                    .unwrap_or(f32::MAX);
                let new_x = (prev_x + delta_x).max(0.0);
                let new_y = (prev_y + delta_y).clamp(0.0, max_y);
                local_scroll.set((new_x, new_y));

                // Repaint at the new scroll position immediately.
                da.queue_draw();

                // Notify engine asynchronously for state tracking and the next full render.
                let tab = tab.borrow().clone();
                TOKIO_RT.spawn(async move {
                    let _ = tab.send(TabCommand::MouseScroll { delta_x, delta_y }).await;
                });
                glib::Propagation::Stop
            }
        });

        // Kinetic (momentum) scrolling: continue scrolling after the finger lifts.
        scroll_ctl.connect_decelerate({
            let tab = tab.clone();
            let local_tiles = local_tiles.clone();
            let local_scroll = local_scroll.clone();
            let kinetic_source = kinetic_source.clone();
            let da = drawing_area.clone();
            move |_ctl, vel_x, vel_y| {
                // vel_x/vel_y are in "scroll units per millisecond" — same units as the
                // dx/dy deltas above, so multiply by 50 to get CSS px/ms.
                let vx = Rc::new(Cell::new(vel_x as f32 * SCROLL_MULTIPLIER));
                let vy = Rc::new(Cell::new(vel_y as f32 * SCROLL_MULTIPLIER));

                let tab = tab.clone();
                let local_tiles = local_tiles.clone();
                let local_scroll = local_scroll.clone();
                let kinetic_source_inner = kinetic_source.clone();
                let da = da.clone();

                // ~60 fps deceleration loop.
                let id = glib::timeout_add_local(std::time::Duration::from_millis(16), move || {
                    let cur_vx = vx.get();
                    let cur_vy = vy.get();
                    if cur_vx.abs() < 2.0 && cur_vy.abs() < 2.0 {
                        *kinetic_source_inner.borrow_mut() = None;
                        return glib::ControlFlow::Break;
                    }
                    // Exponential friction — decelerates to ~5% in ~1 second at 60fps.
                    let friction = 0.93_f32;
                    vx.set(cur_vx * friction);
                    vy.set(cur_vy * friction);

                    let delta_x = cur_vx * 0.016; // velocity × 16ms frame
                    let delta_y = cur_vy * 0.016;

                    let (prev_x, prev_y) = local_scroll.get();
                    let max_y = local_tiles
                        .borrow()
                        .as_ref()
                        .map(|s| (s.page_height - s.viewport_height as f32).max(0.0))
                        .unwrap_or(f32::MAX);
                    local_scroll.set(((prev_x + delta_x).max(0.0), (prev_y + delta_y).clamp(0.0, max_y)));
                    da.queue_draw();

                    let tab = tab.borrow().clone();
                    TOKIO_RT.spawn(async move {
                        let _ = tab.send(TabCommand::MouseScroll { delta_x, delta_y }).await;
                    });

                    glib::ControlFlow::Continue
                });
                *kinetic_source.borrow_mut() = Some(id);
            }
        });
        drawing_area.add_controller(scroll_ctl);

        // Mouse motion → hover
        let motion_ctl = gtk4::EventControllerMotion::new();
        motion_ctl.connect_motion({
            let tab = tab.clone();
            move |_, x, y| {
                let tab = tab.borrow().clone();
                TOKIO_RT.spawn(async move {
                    let _ = tab
                        .send(TabCommand::MouseMove {
                            x: x as f32,
                            y: y as f32,
                        })
                        .await;
                });
            }
        });
        drawing_area.add_controller(motion_ctl);

        // Click → navigate links
        let click_ctl = gtk4::GestureClick::new();
        click_ctl.set_button(gtk4::gdk::BUTTON_PRIMARY);
        click_ctl.connect_pressed({
            let tab = tab.clone();
            move |_, _n_press, x, y| {
                let tab = tab.borrow().clone();
                TOKIO_RT.spawn(async move {
                    let _ = tab
                        .send(TabCommand::MouseDown {
                            x: x as f32,
                            y: y as f32,
                            button: gosub_engine::events::MouseButton::Left,
                        })
                        .await;
                });
            }
        });
        drawing_area.add_controller(click_ctl);

        // Resize → set DPR first so create_surface sees the right value, then notify the engine
        drawing_area.connect_resize({
            let tab = tab.clone();
            let local_tiles = local_tiles.clone();
            let local_scroll = local_scroll.clone();
            move |area, w, h| {
                let _scale = area.scale_factor() as u32;

                // Clear cached tiles — they were rasterized for the old viewport size.
                *local_tiles.borrow_mut() = None;
                local_scroll.set((0.0, 0.0));
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
            let local_tiles = local_tiles.clone();
            let local_scroll = local_scroll.clone();
            let kinetic_source = kinetic_source.clone();
            let da = drawing_area.clone();
            move |entry| {
                // Cancel any kinetic scroll in progress.
                if let Some(id) = kinetic_source.borrow_mut().take() {
                    id.remove();
                }

                let mut s = entry.text().to_string();
                if !s.starts_with("http://") && !s.starts_with("https://") {
                    s = format!("https://{s}");
                    entry.set_text(&s);
                }
                let Ok(url) = Url::parse(&s) else { return };

                // Reset local state for the new page.
                *local_tiles.borrow_mut() = None;
                local_scroll.set((0.0, 0.0));

                let tab = tab.borrow().clone();
                let url_str = url.to_string();
                TOKIO_RT.spawn(async move {
                    let _ = tab.send(TabCommand::Navigate { url: url_str }).await;
                    let _ = tab.send(TabCommand::ResumeDrawing { fps: 30 }).await;
                });
                da.queue_draw();
            }
        });

        // Status bar label (shown at the bottom, like Firefox's link URL preview)
        let status_label = Label::new(None);
        status_label.set_halign(gtk4::Align::Start);
        status_label.set_margin_start(4);
        status_label.set_margin_end(4);
        status_label.set_margin_top(2);
        status_label.set_margin_bottom(2);
        status_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);

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
            let status_label = status_label.clone();
            let address_entry = address_entry.clone();
            let local_tiles = local_tiles.clone();
            let local_scroll = local_scroll.clone();
            let kinetic_source = kinetic_source.clone();
            glib::spawn_future_local(async move {
                while let Some(evt) = ui_rx.recv().await {
                    match evt {
                        EngineEvent::Redraw { .. } => da.queue_draw(),
                        EngineEvent::Navigation { tab_id: _, ref event } => {
                            log::info!("navigation: {event:?}");
                            match event {
                                NavigationEvent::Started { .. } => {
                                    // Same reset the address-bar handler does on manual navigation.
                                    if let Some(id) = kinetic_source.borrow_mut().take() {
                                        id.remove();
                                    }
                                    *local_tiles.borrow_mut() = None;
                                    local_scroll.set((0.0, 0.0));
                                    da.queue_draw();
                                }
                                NavigationEvent::Finished { url, .. } => {
                                    address_entry.set_text(url.as_str());
                                }
                                _ => {}
                            }
                        }
                        EngineEvent::HoverUrl { url, .. } => {
                            status_label.set_text(url.as_deref().unwrap_or(""));
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
        vbox.append(&status_label);

        let window = ApplicationWindow::builder()
            .application(app)
            .title("Gosub Browser — GTK4 + Skia")
            .default_width(1024)
            .default_height(800)
            .child(&vbox)
            .build();

        window.present();

        // Navigate after the window is shown so the viewport is already set.
        {
            let tab_init = tab.clone();
            let url_str = initial_url.clone();
            glib::idle_add_local_once(move || {
                let mut s = url_str.clone();
                if !s.starts_with("http://") && !s.starts_with("https://") {
                    s = format!("https://{s}");
                }
                if let Ok(url) = Url::parse(&s) {
                    let tab = tab_init.borrow().clone();
                    TOKIO_RT.spawn(async move {
                        let _ = tab.send(TabCommand::Navigate { url: url.to_string() }).await;
                        let _ = tab.send(TabCommand::ResumeDrawing { fps: 30 }).await;
                    });
                }
            });
        }
    });

    // Pass only argv[0] to GTK so the URL argument isn't treated as a filename.
    let argv0: Vec<String> = std::env::args().take(1).collect();
    app.run_with_args(&argv0);
}

/// Composite a TileDrawState at the given scroll position into `cr`.
fn draw_tile_cache(cr: &gtk4::cairo::Context, w: i32, h: i32, state: &TileDrawState, scroll_x: f32, scroll_y: f32) {
    let dpr_i = state.dpr as i32;
    let dpr_f = state.dpr as f64;

    let w_phys = w * dpr_i;
    let h_phys = h * dpr_i;

    // CPU-blit tiles into a single image buffer, then paint it once.
    // This avoids per-tile Cairo compositing (which can produce 1-pixel seams at
    // tile boundaries due to bilinear filtering / AA at the source surface edges).
    let Ok(mut dst) = gtk4::cairo::ImageSurface::create(gtk4::cairo::Format::ARgb32, w_phys, h_phys) else {
        return;
    };
    let stride = dst.stride() as usize;

    {
        let Ok(mut data) = dst.data() else {
            return;
        };

        // White background (ARGB32 premultiplied little-endian = 0xFFFF_FFFF).
        for b in data.chunks_exact_mut(4) {
            b[0] = 0xFF;
            b[1] = 0xFF;
            b[2] = 0xFF;
            b[3] = 0xFF;
        }

        for tile in state.tiles.iter() {
            // Viewport position in CSS px from the engine's scroll (handles scroll, fixed and
            // sticky uniformly), then scaled to device px.
            let (vx, vy) = anchored_tile_pos(
                tile.page_x as f64,
                tile.page_y as f64,
                scroll_x as f64,
                scroll_y as f64,
                tile.anchor,
            );
            let px = (vx * dpr_f).round() as i64;
            let py = (vy * dpr_f).round() as i64;
            let tw = tile.width as i64;
            let th = tile.height as i64;

            if px >= w_phys as i64 || py >= h_phys as i64 {
                continue;
            }
            if px + tw <= 0 || py + th <= 0 {
                continue;
            }

            let tile_col0 = (-px).max(0) as usize;
            let tile_row0 = (-py).max(0) as usize;
            let dst_x = px.max(0) as usize;
            let dst_y0 = py.max(0) as usize;
            let tw_usize = tw as usize;
            let th_usize = th as usize;

            for tile_row in tile_row0..th_usize {
                let dst_y = dst_y0 + (tile_row - tile_row0);
                if dst_y >= h_phys as usize {
                    break;
                }
                let copy_w = (tw_usize - tile_col0).min(w_phys as usize - dst_x);
                if copy_w == 0 {
                    break;
                }
                let src_off = (tile_row * tw_usize + tile_col0) * 4;
                let dst_off = dst_y * stride + dst_x * 4;
                // Alpha-blend (source-over) rather than overwrite, so transparent
                // pixels of an upper-layer tile reveal the content drawn beneath it.
                for col in 0..copy_w {
                    let s = src_off + col * 4;
                    let d = dst_off + col * 4;
                    let src_px =
                        u32::from_le_bytes([tile.data[s], tile.data[s + 1], tile.data[s + 2], tile.data[s + 3]]);
                    let src_argb = tile.format.pixel_to_argb_u32(src_px);
                    let dst_px = u32::from_le_bytes([data[d], data[d + 1], data[d + 2], data[d + 3]]);
                    let out = blend_over_argb_u32(scale_premul_argb_u32(src_argb, tile.opacity), dst_px);
                    data[d..d + 4].copy_from_slice(&out.to_le_bytes());
                }
            }
        }
    }

    // Apply device scale so GTK4 maps 1 CSS pixel → dpr physical pixels.
    dst.set_device_scale(dpr_f, dpr_f);

    cr.set_source_surface(&dst, 0.0, 0.0).unwrap_or_default();
    cr.source().set_filter(gtk4::cairo::Filter::Nearest);
    cr.paint().unwrap_or_default();
}

/// Draw a light-grey placeholder while the first frame hasn't arrived yet.
fn draw_placeholder(cr: &gtk4::cairo::Context, w: i32, h: i32) {
    cr.set_source_rgb(0.92, 0.92, 0.92);
    cr.rectangle(0.0, 0.0, w as f64, h as f64);
    cr.fill().unwrap_or_default();
}
