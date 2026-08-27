//! Minimal browser window: Cairo rasterizer + winit toolkit + softbuffer presentation.
//!
//! Usage: cargo run --example winit-cairo -- https://example.com
//!
//! Cairo/Pango need GTK4 initialised for font rendering (no GTK window is created).
//! On headless systems set GDK_BACKEND=offscreen.
//! The page URL is given as the first argument; there is no in-window chrome.

use gosub_engine::events::{EngineEvent, MouseButton, NavigationEvent, TabCommand};
use gosub_engine::storage::{InMemorySessionStore, PartitionPolicy, SqliteLocalStore, StorageService};
use gosub_engine::tab::{TabDefaults, TabHandle, TabId};
use gosub_engine::zone::{Zone, ZoneConfig, ZoneId, ZoneServices};
use gosub_engine::DefaultRenderConfig;
use gosub_engine::GosubEngine;
use gosub_render_pipeline::render::backend::ExternalHandle;
use gosub_render_pipeline::render::DEVICE_PIXEL_RATIO;
use gosub_render_pipeline::render::{composite_tiles, DefaultCompositor, TileTarget};
use gosub_renderer_cairo::{CairoBackend, PangoFontSystem};
use once_cell::sync::Lazy;
use softbuffer::Surface;
use std::num::NonZeroU32;
use std::sync::Arc;
use tokio::runtime::{Builder, Runtime};
use uuid::uuid;
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, MouseButton as WinitMouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy};
use winit::window::{Window, WindowAttributes, WindowId};

const DEFAULT_ZONE: uuid::Uuid = uuid!("f1234567-abcd-4000-8000-000000000006");
const SCROLL_MULTIPLIER: f32 = 134.0;

type AppConfig = DefaultRenderConfig<CairoBackend, PangoFontSystem>;

static TOKIO_RT: Lazy<Runtime> = Lazy::new(|| {
    Builder::new_multi_thread()
        .enable_io()
        .enable_time()
        .thread_name("gosub-winit-cairo-rt")
        .build()
        .expect("tokio runtime")
});

struct BrowserApp {
    // Engine state - set up before the event loop starts.
    #[allow(dead_code)]
    engine: GosubEngine<AppConfig>,
    #[allow(dead_code)]
    zone: Zone<AppConfig>,
    tab: TabHandle,
    tab_id: TabId,
    compositor: Arc<DefaultCompositor>,
    #[allow(dead_code)]
    proxy: EventLoopProxy<()>,

    // Window / surface - created on `resumed`.
    window: Option<Arc<Window>>,
    surface: Option<Surface<Arc<Window>, Arc<Window>>>,
    surface_size: (u32, u32),

    // UI state
    cursor: PhysicalPosition<f64>,
    scroll: (f32, f32),
    page_height: f32,
    viewport: (u32, u32),
}

impl BrowserApp {
    fn new(
        engine: GosubEngine<AppConfig>,
        zone: Zone<AppConfig>,
        tab: TabHandle,
        tab_id: TabId,
        compositor: Arc<DefaultCompositor>,
        proxy: EventLoopProxy<()>,
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
            cursor: PhysicalPosition::default(),
            scroll: (0.0, 0.0),
            page_height: 0.0,
            viewport: (0, 0),
        }
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

        if let Some(handle) = self.compositor.frame_for(self.tab_id) {
            blit_handle_to_buffer(&mut buf, win_w, 0, win_h, self.scroll, handle, &mut self.page_height);
        }
        buf.present().unwrap_or_default();
    }

    /// The device-pixel ratio actually in effect. The Cairo rasterizer scales tiles by the
    /// *integer* `DEVICE_PIXEL_RATIO`, so the viewport and pointer coordinates have to divide by
    /// that same value - using the fractional `scale_factor()` would land them in a third space.
    fn dpr(&self) -> f64 {
        f64::from(DEVICE_PIXEL_RATIO.load(std::sync::atomic::Ordering::Relaxed).max(1))
    }

    /// A physical pixel length as logical (CSS) px, which is the space the engine lays out in.
    fn to_logical(&self, physical: u32) -> u32 {
        ((f64::from(physical) / self.dpr()).round() as u32).max(1)
    }

    fn content_y_to_css(&self, physical_y: f64) -> f32 {
        (physical_y / self.dpr()) as f32
    }

    fn content_x_to_css(&self, physical_x: f64) -> f32 {
        (physical_x / self.dpr()) as f32
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

        // The engine lays out in CSS px; the rasterizer scales tiles up by the DPR. Sending the
        // physical size would lay the page out at DPR times its real size.
        let content_w = self.to_logical(size.width);
        let content_h = self.to_logical(size.height);
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
        // Engine produced a new frame - redraw - and/or asked for a cursor shape.
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

                let content_w = self.to_logical(width);
                let content_h = self.to_logical(height);
                self.viewport = (content_w, content_h);
                self.scroll = (0.0, 0.0);

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
                });

                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }

            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = position;
                let x = self.content_x_to_css(position.x);
                let y = self.content_y_to_css(position.y);
                let tab = self.tab.clone();
                TOKIO_RT.spawn(async move {
                    let _ = tab.send(TabCommand::MouseMove { x, y }).await;
                });
            }

            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: WinitMouseButton::Left,
                ..
            } => {
                let pos = self.cursor;
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

            WindowEvent::MouseInput {
                state: ElementState::Released,
                button: WinitMouseButton::Left,
                ..
            } => {
                let (x, y) = (
                    self.content_x_to_css(self.cursor.x),
                    self.content_y_to_css(self.cursor.y),
                );
                let tab = self.tab.clone();
                TOKIO_RT.spawn(async move {
                    let _ = tab
                        .send(TabCommand::MouseUp {
                            x,
                            y,
                            button: MouseButton::Left,
                        })
                        .await;
                });
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
            // Composite the visible tiles onto the white-filled content region, below the address
            // bar (origin_y = addr_h). softbuffer ignores the high (alpha) byte of each ARGB pixel.
            composite_tiles(
                &tiles,
                tile_dpr,
                (scroll_x, scroll_y),
                &mut TileTarget {
                    buf: &mut buf[..],
                    stride: win_w as usize,
                    origin_x: 0,
                    origin_y: addr_h as usize,
                    width: win_w as usize,
                    height: content_h as usize,
                },
            );
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

fn main() {
    eprintln!(
        "{} v{} — winit browser window, Cairo (CPU) rendering",
        env!("CARGO_BIN_NAME"),
        env!("CARGO_PKG_VERSION")
    );

    simple_logger::SimpleLogger::new()
        .with_level(log::LevelFilter::Warn)
        .env()
        .init()
        .unwrap_or_default();

    // Cairo/Pango need GTK4 initialised. No GTK window is created.
    gosub_renderer_cairo::init_gtk_resources().expect("failed to init GTK resources");

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

    let compositor = Arc::new(DefaultCompositor::new({
        let p = proxy.clone();
        move || {
            let _ = p.send_event(());
        }
    }));

    let backend = CairoBackend::new();
    let mut engine = GosubEngine::<AppConfig>::new(None, Arc::new(backend), compositor.clone());
    // GOSUB_COLOR_SCHEME=dark renders pages and native controls in the dark scheme.
    if std::env::var("GOSUB_COLOR_SCHEME").is_ok_and(|v| v.eq_ignore_ascii_case("dark")) {
        let _ = engine.settings().set(
            "renderer.color_scheme",
            gosub_config::settings::Setting::String("dark".into()),
        );
    }
    let _engine_task = TOKIO_RT.spawn(engine.start().expect("engine start"));

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
        places: None,
    };

    let mut zone = engine
        .create_zone(Some(zone_cfg), zone_services, Some(ZoneId::from(DEFAULT_ZONE)))
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

    let mut app = BrowserApp::new(engine, zone, tab, tab_id, compositor, proxy);

    event_loop.run_app(&mut app).expect("event loop run");
}
