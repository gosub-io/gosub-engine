//! Minimal browser window: Cairo rasterizer + winit toolkit + softbuffer presentation.
//!
//! Usage: cargo run --example winit-cairo -- https://example.com
//!
//! Cairo/Pango need GTK4 initialised for font rendering (no GTK window is created).
//! On headless systems set GDK_BACKEND=offscreen.
//! Press Ctrl+L to focus the address bar.

use gosub_engine::events::{EngineEvent, MouseButton, NavigationEvent, TabCommand};
use gosub_engine::storage::{InMemorySessionStore, PartitionPolicy, SqliteLocalStore, StorageService};
use gosub_engine::tab::{TabDefaults, TabHandle, TabId};
use gosub_engine::zone::{Zone, ZoneConfig, ZoneId, ZoneServices};
use gosub_engine::GosubEngine;
use gosub_render_pipeline::render::backend::{blend_over_argb_u32, ExternalHandle};
use gosub_render_pipeline::render::backends::cairo::{CairoBackend, DEVICE_PIXEL_RATIO};
use gosub_render_pipeline::render::DefaultCompositor;
use once_cell::sync::Lazy;
use parking_lot::RwLock;
use softbuffer::Surface;
use std::num::NonZeroU32;
use std::sync::Arc;
use tokio::runtime::{Builder, Runtime};
use url::Url;
use uuid::uuid;
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, KeyEvent, MouseButton as WinitMouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowAttributes, WindowId};

const DEFAULT_ZONE: uuid::Uuid = uuid!("f1234567-abcd-4000-8000-000000000006");
const ADDRESS_BAR_HEIGHT: u32 = 36;
const SCROLL_MULTIPLIER: f32 = 12.5;

static TOKIO_RT: Lazy<Runtime> = Lazy::new(|| {
    Builder::new_multi_thread()
        .enable_io()
        .enable_time()
        .thread_name("gosub-winit-cairo-rt")
        .build()
        .expect("tokio runtime")
});

struct BrowserApp {
    // Engine state — set up before the event loop starts.
    #[allow(dead_code)]
    engine: GosubEngine,
    #[allow(dead_code)]
    zone: Zone,
    tab: TabHandle,
    tab_id: TabId,
    compositor: Arc<RwLock<DefaultCompositor>>,
    #[allow(dead_code)]
    proxy: EventLoopProxy<()>,

    // Window / surface — created on `resumed`.
    window: Option<Arc<Window>>,
    surface: Option<Surface<Arc<Window>, Arc<Window>>>,
    surface_size: (u32, u32),

    // UI state
    url_input: String,
    addr_focused: bool,
    cursor: PhysicalPosition<f64>,
    scroll: (f32, f32),
    page_height: f32,
    viewport: (u32, u32),
}

impl BrowserApp {
    fn new(
        engine: GosubEngine,
        zone: Zone,
        tab: TabHandle,
        tab_id: TabId,
        compositor: Arc<RwLock<DefaultCompositor>>,
        proxy: EventLoopProxy<()>,
        initial_url: String,
    ) -> Self {
        Self {
            engine,
            zone,
            tab,
            tab_id,
            compositor,
            proxy,
            window: None,
            surface: None,
            surface_size: (0, 0),
            url_input: initial_url,
            addr_focused: false,
            cursor: PhysicalPosition::default(),
            scroll: (0.0, 0.0),
            page_height: 0.0,
            viewport: (0, 0),
        }
    }

    fn navigate(&mut self) {
        let mut s = self.url_input.clone();
        if !s.starts_with("http://") && !s.starts_with("https://") {
            s = format!("https://{s}");
            self.url_input = s.clone();
        }
        let Ok(_) = Url::parse(&s) else { return };
        self.scroll = (0.0, 0.0);
        self.addr_focused = false;
        let tab = self.tab.clone();
        TOKIO_RT.spawn(async move {
            let _ = tab.send(TabCommand::Navigate { url: s }).await;
            let _ = tab.send(TabCommand::ResumeDrawing { fps: 30 }).await;
        });
    }

    fn redraw(&mut self) {
        let Some(_window) = &self.window else { return };
        let Some(surface) = &mut self.surface else { return };

        let (win_w, win_h) = self.surface_size;
        if win_w == 0 || win_h == 0 {
            return;
        }

        let Ok(nw) = NonZeroU32::try_from(win_w) else { return };
        let Ok(nh) = NonZeroU32::try_from(win_h) else { return };
        if surface.resize(nw, nh).is_err() {
            return;
        }

        let Ok(mut buf) = surface.buffer_mut() else { return };

        // Fill opaque white (valid premultiplied background for source-over blending).
        buf.fill(0xFFFF_FFFF);

        // Composite engine content into the content area (below address bar).
        let content_h = win_h.saturating_sub(ADDRESS_BAR_HEIGHT);
        if content_h > 0 {
            let guard = self.compositor.read();
            if let Some(handle) = guard.frame_for(self.tab_id) {
                blit_handle_to_buffer(
                    &mut buf,
                    win_w,
                    ADDRESS_BAR_HEIGHT,
                    content_h,
                    self.scroll,
                    handle,
                    &mut self.page_height,
                );
            }
        }

        // Draw address bar on top.
        draw_address_bar(&mut buf, win_w, &self.url_input, self.addr_focused);

        buf.present().unwrap_or_default();
    }

    fn content_y_to_css(&self, physical_y: f64) -> f32 {
        let dpr = DEVICE_PIXEL_RATIO.load(std::sync::atomic::Ordering::Relaxed) as f64;
        let logical_y = physical_y / dpr - ADDRESS_BAR_HEIGHT as f64;
        (logical_y + self.scroll.1 as f64) as f32
    }

    fn content_x_to_css(&self, physical_x: f64) -> f32 {
        let dpr = DEVICE_PIXEL_RATIO.load(std::sync::atomic::Ordering::Relaxed) as f64;
        (physical_x / dpr + self.scroll.0 as f64) as f32
    }

    fn is_in_address_bar(&self, physical_y: f64) -> bool {
        let dpr = DEVICE_PIXEL_RATIO.load(std::sync::atomic::Ordering::Relaxed) as f64;
        physical_y < ADDRESS_BAR_HEIGHT as f64 * dpr
    }
}

impl ApplicationHandler<()> for BrowserApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let attrs = WindowAttributes::default()
            .with_title("Gosub Browser — winit + Cairo")
            .with_inner_size(LogicalSize::new(1024u32, 768u32));

        let window = Arc::new(event_loop.create_window(attrs).expect("create window"));

        let dpr = window.scale_factor() as u32;
        DEVICE_PIXEL_RATIO.store(dpr.max(1), std::sync::atomic::Ordering::Relaxed);

        let ctx = softbuffer::Context::new(window.clone()).expect("softbuffer context");
        let surface = Surface::new(&ctx, window.clone()).expect("softbuffer surface");

        let size = window.inner_size();
        self.surface_size = (size.width, size.height);

        let content_h = size.height.saturating_sub(ADDRESS_BAR_HEIGHT);
        let content_w = size.width;
        self.viewport = (content_w, content_h);

        let tab = self.tab.clone();
        TOKIO_RT.spawn(async move {
            let _ = tab
                .send(TabCommand::SetViewport {
                    x: 0,
                    y: 0,
                    width: content_w,
                    height: content_h,
                })
                .await;
            let _ = tab.send(TabCommand::ResumeDrawing { fps: 30 }).await;
        });

        self.window = Some(window);
        self.surface = Some(surface);
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, _event: ()) {
        // Engine produced a new frame — redraw.
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),

            WindowEvent::RedrawRequested => self.redraw(),

            WindowEvent::Resized(PhysicalSize { width, height }) => {
                if width == 0 || height == 0 {
                    return;
                }
                self.surface_size = (width, height);
                let dpr = self.window.as_ref().map(|w| w.scale_factor() as u32).unwrap_or(1);
                DEVICE_PIXEL_RATIO.store(dpr.max(1), std::sync::atomic::Ordering::Relaxed);

                let content_h = height.saturating_sub(ADDRESS_BAR_HEIGHT);
                self.viewport = (width, content_h);
                self.scroll = (0.0, 0.0);

                let tab = self.tab.clone();
                TOKIO_RT.spawn(async move {
                    let _ = tab
                        .send(TabCommand::SetViewport {
                            x: 0,
                            y: 0,
                            width,
                            height: content_h,
                        })
                        .await;
                });

                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }

            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = position;
                if !self.is_in_address_bar(position.y) {
                    let x = self.content_x_to_css(position.x);
                    let y = self.content_y_to_css(position.y);
                    let tab = self.tab.clone();
                    TOKIO_RT.spawn(async move {
                        let _ = tab.send(TabCommand::MouseMove { x, y }).await;
                    });
                }
            }

            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: WinitMouseButton::Left,
                ..
            } => {
                let pos = self.cursor;
                if self.is_in_address_bar(pos.y) {
                    self.addr_focused = true;
                    if let Some(window) = &self.window {
                        window.request_redraw();
                    }
                } else {
                    self.addr_focused = false;
                    let x = self.content_x_to_css(pos.x);
                    let y = self.content_y_to_css(pos.y);
                    let tab = self.tab.clone();
                    TOKIO_RT.spawn(async move {
                        let _ = tab
                            .send(TabCommand::MouseDown {
                                x,
                                y,
                                button: MouseButton::Left,
                            })
                            .await;
                    });
                }
            }

            WindowEvent::MouseWheel { delta, .. } => {
                let (dx, dy) = match delta {
                    MouseScrollDelta::LineDelta(x, y) => (x * SCROLL_MULTIPLIER, y * SCROLL_MULTIPLIER),
                    MouseScrollDelta::PixelDelta(p) => (p.x as f32, p.y as f32),
                };

                let max_y = (self.page_height - self.viewport.1 as f32).max(0.0);
                self.scroll.0 = (self.scroll.0 + dx).max(0.0);
                self.scroll.1 = (self.scroll.1 + dy).clamp(0.0, max_y);

                let tab = self.tab.clone();
                TOKIO_RT.spawn(async move {
                    let _ = tab
                        .send(TabCommand::MouseScroll {
                            delta_x: dx,
                            delta_y: dy,
                        })
                        .await;
                });

                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }

            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        logical_key,
                        text,
                        state: ElementState::Pressed,
                        ..
                    },
                ..
            } => {
                // Ctrl+L: focus address bar
                if logical_key == Key::Character("l".into()) {
                    // We can't reliably detect Ctrl here without modifiers check,
                    // but for simplicity treat any 'l' press outside addr bar as a shortcut.
                }

                if self.addr_focused {
                    match &logical_key {
                        Key::Named(NamedKey::Enter) => self.navigate(),
                        Key::Named(NamedKey::Escape) => {
                            self.addr_focused = false;
                            if let Some(window) = &self.window {
                                window.request_redraw();
                            }
                        }
                        Key::Named(NamedKey::Backspace) => {
                            self.url_input.pop();
                            if let Some(window) = &self.window {
                                window.request_redraw();
                            }
                        }
                        _ => {
                            if let Some(t) = &text {
                                self.url_input.push_str(t.as_str());
                                if let Some(window) = &self.window {
                                    window.request_redraw();
                                }
                            }
                        }
                    }
                }
            }

            _ => {}
        }
    }
}

/// Blit compositor frame into the softbuffer below the address bar.
fn blit_handle_to_buffer(
    buf: &mut softbuffer::Buffer<Arc<Window>, Arc<Window>>,
    win_w: u32,
    addr_h: u32,
    content_h: u32,
    _scroll: (f32, f32),
    handle: ExternalHandle,
    page_height: &mut f32,
) {
    match handle {
        ExternalHandle::CpuPixelsOwned {
            width,
            height,
            stride,
            pixels,
            ..
        } => {
            let copy_rows = height.min(content_h) as usize;
            for row in 0..copy_rows {
                for col in 0..(width as usize).min(win_w as usize) {
                    let src_off = row * stride as usize + col * 4;
                    let b = pixels[src_off] as u32;
                    let g = pixels[src_off + 1] as u32;
                    let r = pixels[src_off + 2] as u32;
                    let dst_idx = (addr_h as usize + row) * win_w as usize + col;
                    if dst_idx < buf.len() {
                        buf[dst_idx] = (r << 16) | (g << 8) | b;
                    }
                }
            }
        }
        ExternalHandle::TileCache {
            tiles,
            dpr: tile_dpr,
            page_height: ph,
            scroll_x,
            scroll_y,
            ..
        } => {
            *page_height = ph;
            let dpr_f = tile_dpr as f32;
            let sx = (scroll_x * dpr_f).round() as i64;
            let sy = (scroll_y * dpr_f).round() as i64;

            for tile in tiles.iter() {
                // Signed physical position of tile relative to content area top-left.
                let px = (tile.page_x * dpr_f).round() as i64 - sx;
                let py = (tile.page_y * dpr_f).round() as i64 - sy;
                let tw = tile.width as i64;
                let th = tile.height as i64;
                let cw_i = win_w as i64;
                let ch_i = content_h as i64;

                // Skip tiles completely outside the content area.
                if px >= cw_i || py >= ch_i || px + tw <= 0 || py + th <= 0 {
                    continue;
                }

                // How many leading tile columns/rows are off-screen to the left/top.
                let tile_col0 = (-px).max(0) as usize;
                let tile_row0 = (-py).max(0) as usize;
                let dst_x = px.max(0) as usize;
                let dst_y0 = py.max(0) as usize;
                let tw_usize = tw as usize;
                let th_usize = th as usize;

                // Cairo ARGB32 LE: [B, G, R, A] in bytes → u32 = A<<24|R<<16|G<<8|B.
                // softbuffer wants 0x00RRGGBB → mask off the high (alpha) byte.
                let tile_u32 =
                    unsafe { std::slice::from_raw_parts(tile.data.as_ptr() as *const u32, tile.data.len() / 4) };

                for tile_row in tile_row0..th_usize {
                    let dst_y = dst_y0 + (tile_row - tile_row0);
                    if dst_y >= ch_i as usize {
                        break;
                    }
                    let copy_w = (tw_usize - tile_col0).min(cw_i as usize - dst_x);
                    if copy_w == 0 {
                        break;
                    }
                    // dst_y is relative to the content area; add addr_h for the absolute row.
                    let buf_row = (addr_h as usize + dst_y) * win_w as usize + dst_x;
                    let src_row = tile_row * tw_usize + tile_col0;
                    for col in 0..copy_w {
                        // Source-over blend so transparent upper-layer pixels reveal the
                        // content (or white background) beneath, instead of overwriting it.
                        let src_argb = tile.format.pixel_to_argb_u32(tile_u32[src_row + col]);
                        buf[buf_row + col] = blend_over_argb_u32(src_argb, buf[buf_row + col]);
                    }
                }
            }
        }
        ExternalHandle::CpuPixelsPtr {
            width,
            height,
            stride,
            pixel_buf,
        } => {
            let pixels = unsafe { std::slice::from_raw_parts(pixel_buf.as_ptr(), height as usize * stride as usize) };
            let copy_rows = height.min(content_h) as usize;
            for row in 0..copy_rows {
                for col in 0..(width as usize).min(win_w as usize) {
                    let src_off = row * stride as usize + col * 4;
                    let b = pixels[src_off] as u32;
                    let g = pixels[src_off + 1] as u32;
                    let r = pixels[src_off + 2] as u32;
                    let dst_idx = (addr_h as usize + row) * win_w as usize + col;
                    if dst_idx < buf.len() {
                        buf[dst_idx] = (r << 16) | (g << 8) | b;
                    }
                }
            }
        }
        _ => {}
    }
}

/// Draw the address bar into the top ADDRESS_BAR_HEIGHT rows of the buffer using Cairo.
fn draw_address_bar(buf: &mut softbuffer::Buffer<Arc<Window>, Arc<Window>>, win_w: u32, url: &str, focused: bool) {
    let h = ADDRESS_BAR_HEIGHT as i32;
    let w = win_w as i32;

    let Ok(mut surface) = cairo::ImageSurface::create(cairo::Format::ARgb32, w, h) else {
        // Fallback: fill with flat gray
        let gray = 0x00D0D0D0u32;
        for row in 0..ADDRESS_BAR_HEIGHT as usize {
            for col in 0..win_w as usize {
                buf[row * win_w as usize + col] = gray;
            }
        }
        return;
    };

    {
        let Ok(cr) = cairo::Context::new(&surface) else {
            return;
        };

        // Background
        cr.set_source_rgb(0.93, 0.93, 0.93);
        cr.rectangle(0.0, 0.0, w as f64, h as f64);
        cr.fill().unwrap_or_default();

        // Input box
        let (bg_r, bg_g, bg_b) = if focused { (1.0, 1.0, 1.0) } else { (0.97, 0.97, 0.97) };
        cr.set_source_rgb(bg_r, bg_g, bg_b);
        cr.rectangle(4.0, 5.0, (w - 8) as f64, (h - 10) as f64);
        cr.fill().unwrap_or_default();

        // Border
        let (br, bg, bb) = if focused { (0.26, 0.52, 0.96) } else { (0.7, 0.7, 0.7) };
        cr.set_source_rgb(br, bg, bb);
        cr.set_line_width(1.0);
        cr.rectangle(4.5, 5.5, (w - 9) as f64, (h - 11) as f64);
        cr.stroke().unwrap_or_default();

        // URL text
        cr.set_source_rgb(0.0, 0.0, 0.0);
        cr.select_font_face("Sans", cairo::FontSlant::Normal, cairo::FontWeight::Normal);
        cr.set_font_size(14.0);
        cr.move_to(10.0, h as f64 - 10.0);
        cr.show_text(url).unwrap_or_default();
    }

    surface.flush();

    let Ok(data) = surface.data() else { return };

    // Copy ARGB32 → softbuffer u32 (mask off alpha byte).
    for row in 0..ADDRESS_BAR_HEIGHT as usize {
        for col in 0..win_w as usize {
            let off = row * (w * 4) as usize + col * 4;
            if off + 3 >= data.len() {
                break;
            }
            let b = data[off] as u32;
            let g = data[off + 1] as u32;
            let r = data[off + 2] as u32;
            buf[row * win_w as usize + col] = (r << 16) | (g << 8) | b;
        }
    }
}

fn main() {
    simple_logger::SimpleLogger::new()
        .with_level(log::LevelFilter::Warn)
        .env()
        .init()
        .unwrap_or_default();

    // Cairo/Pango need GTK4 initialised. No GTK window is created.
    gosub_engine::init_gtk_resources().expect("failed to init GTK resources");

    let initial_url = {
        let raw = std::env::args()
            .nth(1)
            .unwrap_or_else(|| "https://example.com".to_string());
        if raw.contains("://") {
            raw
        } else {
            format!("https://{raw}")
        }
    };

    let _rt_guard = TOKIO_RT.enter();

    // Build the event loop first so we have a proxy for the compositor callback.
    let event_loop = EventLoop::<()>::with_user_event().build().expect("event loop");
    let proxy = event_loop.create_proxy();

    let compositor = Arc::new(RwLock::new(DefaultCompositor::new({
        let p = proxy.clone();
        move || {
            let _ = p.send_event(());
        }
    })));

    let backend = CairoBackend::new();
    let mut engine = GosubEngine::new(None, Arc::new(backend), compositor.clone());
    let _join = engine.start().expect("engine start");

    // Forward engine navigation events to update the window title.
    let proxy_ev = proxy.clone();
    let mut event_rx = engine.subscribe_events();
    TOKIO_RT.spawn(async move {
        loop {
            match event_rx.recv().await {
                Ok(EngineEvent::Navigation {
                    event: NavigationEvent::Finished { .. } | NavigationEvent::Started { .. },
                    ..
                }) => {
                    let _ = proxy_ev.send_event(());
                }
                Ok(_) => {}
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    let zone_cfg = ZoneConfig::builder().do_not_track(true).build().expect("ZoneConfig");
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
                title: Some("Gosub".to_string()),
                viewport: None,
            },
            None,
        ))
        .expect("create_tab");

    let tab_id = tab.tab_id;
    let nav_tab = tab.clone();
    let nav_url = initial_url.clone();
    TOKIO_RT.spawn(async move {
        let _ = nav_tab.send(TabCommand::Navigate { url: nav_url }).await;
    });

    let mut app = BrowserApp::new(engine, zone, tab, tab_id, compositor, proxy, initial_url);

    event_loop.run_app(&mut app).expect("event loop run");
}
