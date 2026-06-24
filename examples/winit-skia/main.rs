//! Minimal browser window: Skia (CPU) rasterizer + winit toolkit + softbuffer presentation.
//!
//! Usage: cargo run --example winit-skia -- https://example.com
//!
//! No GTK dependency — Skia has its own font system.
//! Press Ctrl+L to focus the address bar.

use gosub_engine::events::{EngineEvent, MouseButton, NavigationEvent, TabCommand};
use gosub_engine::storage::{InMemorySessionStore, PartitionPolicy, SqliteLocalStore, StorageService};
use gosub_engine::tab::{TabDefaults, TabHandle, TabId};
use gosub_engine::zone::{Zone, ZoneConfig, ZoneId, ZoneServices};
use gosub_engine::DefaultRenderConfig;
use gosub_engine::GosubEngine;
use gosub_render_pipeline::render::backend::{blend_over_argb_u32, scale_premul_argb_u32, ExternalHandle};
use gosub_render_pipeline::render::DefaultCompositor;
use gosub_renderer_skia::{SkiaBackend, SkiaFontSystem};
use once_cell::sync::Lazy;
use parking_lot::RwLock;
use skia_safe::{surfaces, Color4f, Font, FontMgr, FontStyle, Paint, Rect as SkRect};
use softbuffer::Surface;
use std::num::NonZeroU32;
use std::sync::Arc;
use tokio::runtime::{Builder, Runtime};
use tokio::sync::watch;
use url::Url;
use uuid::uuid;
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, KeyEvent, MouseButton as WinitMouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowAttributes, WindowId};

const DEFAULT_ZONE: uuid::Uuid = uuid!("f1234567-abcd-4000-8000-00000000000a");
const ADDRESS_BAR_HEIGHT: u32 = 36;
const SCROLL_MULTIPLIER: f32 = 12.5;

type AppConfig = DefaultRenderConfig<SkiaBackend, SkiaFontSystem>;

static TOKIO_RT: Lazy<Runtime> = Lazy::new(|| {
    Builder::new_multi_thread()
        .enable_io()
        .enable_time()
        .thread_name("gosub-winit-skia-rt")
        .build()
        .expect("tokio runtime")
});

struct BrowserApp {
    #[allow(dead_code)]
    engine: GosubEngine<AppConfig>,
    #[allow(dead_code)]
    zone: Zone<AppConfig>,
    tab: TabHandle,
    tab_id: TabId,
    compositor: Arc<RwLock<DefaultCompositor>>,
    #[allow(dead_code)]
    proxy: EventLoopProxy<()>,
    mouse_tx: watch::Sender<Option<(f32, f32)>>,

    window: Option<Arc<Window>>,
    surface: Option<Surface<Arc<Window>, Arc<Window>>>,
    surface_size: (u32, u32),

    url_input: String,
    addr_focused: bool,
    cursor: PhysicalPosition<f64>,
    scroll: (f32, f32),
    page_height: f32,
    viewport: (u32, u32),
}

impl BrowserApp {
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
        // Opaque white: a valid premultiplied background for source-over blending.
        buf.fill(0xFFFF_FFFF);

        let content_h = win_h.saturating_sub(ADDRESS_BAR_HEIGHT);
        if content_h > 0 {
            let guard = self.compositor.read();
            if let Some(handle) = guard.frame_for(self.tab_id) {
                blit_to_buffer(
                    &mut buf,
                    win_w,
                    ADDRESS_BAR_HEIGHT,
                    content_h,
                    handle,
                    &mut self.page_height,
                    self.scroll,
                );
            }
        }

        draw_address_bar(&mut buf, win_w, &self.url_input, self.addr_focused);
        buf.present().unwrap_or_default();
    }

    fn is_in_address_bar(&self, y: f64) -> bool {
        y < ADDRESS_BAR_HEIGHT as f64
    }

    fn to_css_x(&self, x: f64) -> f32 {
        x as f32
    }

    fn to_css_y(&self, y: f64) -> f32 {
        (y - ADDRESS_BAR_HEIGHT as f64) as f32
    }
}

impl ApplicationHandler<()> for BrowserApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let attrs = WindowAttributes::default()
            .with_title("Gosub Browser — winit + Skia")
            .with_inner_size(LogicalSize::new(1024u32, 768u32));

        let window = Arc::new(event_loop.create_window(attrs).expect("create window"));
        let ctx = softbuffer::Context::new(window.clone()).expect("softbuffer context");
        let surface = Surface::new(&ctx, window.clone()).expect("softbuffer surface");

        let size = window.inner_size();
        self.surface_size = (size.width, size.height);
        let content_h = size.height.saturating_sub(ADDRESS_BAR_HEIGHT);
        self.viewport = (size.width, content_h);

        let tab = self.tab.clone();
        TOKIO_RT.spawn(async move {
            let _ = tab
                .send(TabCommand::SetViewport {
                    x: 0,
                    y: 0,
                    width: size.width,
                    height: content_h,
                })
                .await;
            let _ = tab.send(TabCommand::ResumeDrawing { fps: 30 }).await;
        });

        self.window = Some(window);
        self.surface = Some(surface);
    }

    fn user_event(&mut self, _: &ActiveEventLoop, _: ()) {
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::RedrawRequested => self.redraw(),

            WindowEvent::Resized(PhysicalSize { width, height }) => {
                if width == 0 || height == 0 {
                    return;
                }
                self.surface_size = (width, height);
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
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }

            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = position;
                if !self.is_in_address_bar(position.y) {
                    let (x, y) = (self.to_css_x(position.x), self.to_css_y(position.y));
                    let _ = self.mouse_tx.send(Some((x, y)));
                }
            }

            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: WinitMouseButton::Left,
                ..
            } => {
                if self.is_in_address_bar(self.cursor.y) {
                    self.addr_focused = true;
                    if let Some(w) = &self.window {
                        w.request_redraw();
                    }
                } else {
                    self.addr_focused = false;
                    let (x, y) = (self.to_css_x(self.cursor.x), self.to_css_y(self.cursor.y));
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
                if let Some(w) = &self.window {
                    w.request_redraw();
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
            } if self.addr_focused => match &logical_key {
                Key::Named(NamedKey::Enter) => self.navigate(),
                Key::Named(NamedKey::Escape) => {
                    self.addr_focused = false;
                    if let Some(w) = &self.window {
                        w.request_redraw();
                    }
                }
                Key::Named(NamedKey::Backspace) => {
                    self.url_input.pop();
                    if let Some(w) = &self.window {
                        w.request_redraw();
                    }
                }
                _ => {
                    if let Some(t) = &text {
                        self.url_input.push_str(t.as_str());
                        if let Some(w) = &self.window {
                            w.request_redraw();
                        }
                    }
                }
            },

            _ => {}
        }
    }
}

fn blit_to_buffer(
    buf: &mut softbuffer::Buffer<Arc<Window>, Arc<Window>>,
    win_w: u32,
    addr_h: u32,
    content_h: u32,
    handle: ExternalHandle,
    page_height: &mut f32,
    scroll: (f32, f32),
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
            let cols = (width as usize).min(win_w as usize);
            for row in 0..copy_rows {
                let dst_base = (addr_h as usize + row) * win_w as usize;
                for col in 0..cols {
                    let off = row * stride as usize + col * 4;
                    let b = pixels[off] as u32;
                    let g = pixels[off + 1] as u32;
                    let r = pixels[off + 2] as u32;
                    buf[dst_base + col] = (r << 16) | (g << 8) | b;
                }
            }
        }
        ExternalHandle::TileCache {
            tiles,
            page_height: ph,
            dpr,
            ..
        } => {
            *page_height = ph;
            let d = dpr as usize;
            // Use the locally-tracked scroll position so scrolling feels instant without
            // waiting for the engine to acknowledge the scroll command.
            let sx = scroll.0 as usize * d;
            let sy = scroll.1 as usize * d;
            for tile in tiles.iter() {
                let tile_px = tile.page_x as usize * d;
                let tile_py = tile.page_y as usize * d;
                let tw = tile.width as usize;
                let th = tile.height as usize;

                // Skip tiles entirely outside the viewport.
                if tile_px + tw <= sx || tile_py + th <= sy {
                    continue;
                }

                // First visible row/col within the tile (> 0 when tile is partially above/left).
                let row_start = sy.saturating_sub(tile_py);
                let col_start = sx.saturating_sub(tile_px);

                // Screen position of the first visible pixel.
                let screen_x = tile_px.saturating_sub(sx);
                let screen_y = tile_py.saturating_sub(sy);

                let tile_u32 =
                    unsafe { std::slice::from_raw_parts(tile.data.as_ptr() as *const u32, tile.data.len() / 4) };

                for row in row_start..th {
                    let dst_y = addr_h as usize + screen_y + (row - row_start);
                    if dst_y >= (addr_h + content_h) as usize {
                        break;
                    }
                    let visible_cols = tw - col_start;
                    let avail_x = win_w as usize - screen_x.min(win_w as usize);
                    let cw = visible_cols.min(avail_x);
                    if cw == 0 {
                        break;
                    }
                    // Skia tiles are BGRA8888 (premultiplied); reinterpreted as a
                    // little-endian u32 they read as 0xAARRGGBB. Source-over blend so
                    // transparent upper-layer pixels reveal the content beneath instead
                    // of overwriting it. softbuffer ignores the high (alpha) byte.
                    let row_off = row * tw + col_start;
                    let src = &tile_u32[row_off..row_off + cw];
                    let dst_base = dst_y * win_w as usize + screen_x;
                    let dst = &mut buf[dst_base..dst_base + cw];
                    for (d, &s) in dst.iter_mut().zip(src.iter()) {
                        let src_argb = tile.format.pixel_to_argb_u32(s);
                        *d = blend_over_argb_u32(scale_premul_argb_u32(src_argb, tile.opacity), *d);
                    }
                }
            }
        }
        _ => {}
    }
}

/// Draw the address bar using Skia directly into the top ADDRESS_BAR_HEIGHT rows.
fn draw_address_bar(buf: &mut softbuffer::Buffer<Arc<Window>, Arc<Window>>, win_w: u32, url: &str, focused: bool) {
    let h = ADDRESS_BAR_HEIGHT as i32;
    let w = win_w as i32;

    let Some(mut surface) = surfaces::raster_n32_premul(skia_safe::ISize::new(w, h)) else {
        buf[..ADDRESS_BAR_HEIGHT as usize * win_w as usize].fill(0x00D0D0D0);
        return;
    };

    let canvas = surface.canvas();

    canvas.clear(if focused {
        Color4f::new(0.98, 0.98, 0.98, 1.0)
    } else {
        Color4f::new(0.93, 0.93, 0.93, 1.0)
    });

    let box_color = if focused {
        Color4f::new(1.0, 1.0, 1.0, 1.0)
    } else {
        Color4f::new(0.97, 0.97, 0.97, 1.0)
    };
    let mut bg = Paint::new(box_color, None);
    bg.set_anti_alias(true);
    canvas.draw_rect(SkRect::new(4.0, 5.0, (w - 4) as f32, (h - 5) as f32), &bg);

    let border_color = if focused {
        Color4f::new(0.26, 0.52, 0.96, 1.0)
    } else {
        Color4f::new(0.7, 0.7, 0.7, 1.0)
    };
    let mut border = Paint::new(border_color, None);
    border.set_anti_alias(true);
    border.set_style(skia_safe::PaintStyle::Stroke);
    border.set_stroke_width(1.0);
    canvas.draw_rect(SkRect::new(4.5, 5.5, (w - 5) as f32, (h - 5) as f32), &border);

    thread_local! { static FONT_MGR: FontMgr = FontMgr::new(); }
    let typeface = FONT_MGR.with(|fm| {
        fm.legacy_make_typeface(None, FontStyle::normal()).unwrap_or_else(|| {
            fm.legacy_make_typeface("sans-serif", FontStyle::normal())
                .expect("no typeface")
        })
    });
    let font = Font::new(typeface, 14.0);
    let mut text_paint = Paint::new(Color4f::new(0.0, 0.0, 0.0, 1.0), None);
    text_paint.set_anti_alias(true);
    canvas.draw_str(url, (10.0f32, h as f32 - 10.0), &font, &text_paint);

    if let Some(peek) = canvas.peek_pixels() {
        if let Some(bytes) = peek.bytes() {
            for row in 0..ADDRESS_BAR_HEIGHT as usize {
                for col in 0..win_w as usize {
                    let off = row * win_w as usize * 4 + col * 4;
                    if off + 2 < bytes.len() {
                        let b = bytes[off] as u32;
                        let g = bytes[off + 1] as u32;
                        let r = bytes[off + 2] as u32;
                        buf[row * win_w as usize + col] = (r << 16) | (g << 8) | b;
                    }
                }
            }
        }
    }
}

fn main() {
    simple_logger::SimpleLogger::new()
        .with_level(log::LevelFilter::Warn)
        .env()
        .init()
        .unwrap_or_default();

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
    let event_loop = EventLoop::<()>::with_user_event().build().expect("event loop");
    let proxy = event_loop.create_proxy();

    let compositor = Arc::new(RwLock::new(DefaultCompositor::new({
        let p = proxy.clone();
        move || {
            let _ = p.send_event(());
        }
    })));

    let backend = SkiaBackend::new();
    let mut engine = GosubEngine::<AppConfig>::new(None, Arc::new(backend), compositor.clone());
    let _join = engine.start().expect("engine start");

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

    let (mouse_tx, mut mouse_rx) = watch::channel::<Option<(f32, f32)>>(None);
    let hover_tab = tab.clone();
    TOKIO_RT.spawn(async move {
        while mouse_rx.changed().await.is_ok() {
            // Copy the value out before awaiting so the Ref guard is dropped.
            let pos = *mouse_rx.borrow_and_update();
            if let Some((x, y)) = pos {
                let _ = hover_tab.send(TabCommand::MouseMove { x, y }).await;
            }
        }
    });

    let mut app = BrowserApp {
        engine,
        zone,
        tab,
        tab_id,
        compositor,
        proxy,
        mouse_tx,
        window: None,
        surface: None,
        surface_size: (0, 0),
        url_input: initial_url,
        addr_focused: false,
        cursor: PhysicalPosition::default(),
        scroll: (0.0, 0.0),
        page_height: 0.0,
        viewport: (1024, 768),
    };

    event_loop.run_app(&mut app).expect("event loop");
}
