//! Minimal browser window: Skia GPU (OpenGL/Ganesh) rasterizer + winit.
//!
//! Usage: cargo run --example winit-skia-gpu -- https://example.com
//!
//! Uses glutin to create an OpenGL context, then Skia's Ganesh GPU backend
//! to rasterise into an offscreen FBO.  The result is read back to CPU and
//! presented via softbuffer — so the compositing step stays identical to the
//! CPU Skia example, while rasterisation benefits from GPU acceleration.
//!
//! Requires ninja + clang to build skia-bindings with the `gl` feature.

// Link against libGL for OpenGL symbols used by Skia's Ganesh backend.
#[link(name = "GL")]
extern "C" {}

use glutin::config::{Config, GlConfig};
use glutin::context::{ContextApi, ContextAttributesBuilder, NotCurrentGlContext, PossiblyCurrentContext};
use glutin::display::GetGlDisplay;
use glutin::prelude::GlDisplay;
use glutin::surface::{Surface as GlSurface_, WindowSurface};
use glutin_winit::{DisplayBuilder, GlWindow};
use gosub_engine::events::{EngineEvent, MouseButton, NavigationEvent, TabCommand};
use gosub_engine::storage::{InMemorySessionStore, PartitionPolicy, SqliteLocalStore, StorageService};
use gosub_engine::tab::{TabDefaults, TabHandle, TabId};
use gosub_engine::zone::{Zone, ZoneConfig, ZoneId, ZoneServices};
use gosub_engine::GosubEngine;
use gosub_render_pipeline::render::backend::ExternalHandle;
use gosub_render_pipeline::render::backends::skia_gpu::{GlContextProvider, SkiaGpuBackend};
use gosub_render_pipeline::render::DefaultCompositor;
use once_cell::sync::Lazy;
use parking_lot::RwLock;
use skia_safe::{surfaces, Color4f, Font, FontMgr, FontStyle, Paint, Rect as SkRect};
use softbuffer::Surface;
use std::ffi::CString;
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
use winit::raw_window_handle::HasWindowHandle;
use winit::window::{Window, WindowAttributes, WindowId};

const DEFAULT_ZONE: uuid::Uuid = uuid!("f1234567-abcd-4000-8000-00000000000c");
const ADDRESS_BAR_HEIGHT: u32 = 36;
const SCROLL_MULTIPLIER: f32 = 12.5;

static TOKIO_RT: Lazy<Runtime> = Lazy::new(|| {
    Builder::new_multi_thread()
        .enable_io()
        .enable_time()
        .thread_name("gosub-winit-skia-gpu-rt")
        .build()
        .expect("tokio runtime")
});

// ── GlContextProvider impl for glutin ────────────────────────────────────────

struct GlutinContext {
    gl_config: Config,
    #[allow(dead_code)]
    context: PossiblyCurrentContext,
    #[allow(dead_code)]
    gl_surface: GlSurface_<WindowSurface>,
}

// SAFETY: EGL/GLX contexts can be made current on any thread.
// Our Mutex<SendDirectContext> in SkiaGpuBackend ensures single-threaded access.
unsafe impl Send for GlutinContext {}
unsafe impl Sync for GlutinContext {}

impl GlContextProvider for GlutinContext {
    fn make_current(&self) {
        // Context was made current at init and we keep it on the same thread.
        // If the engine rendering thread changes, this would need to re-make-current.
    }

    fn get_proc_address(&self, name: &str) -> *const std::ffi::c_void {
        let c_name = CString::new(name).unwrap_or_default();
        self.gl_config.display().get_proc_address(&c_name)
    }
}

// ── Application ───────────────────────────────────────────────────────────────

struct BrowserApp {
    #[allow(dead_code)]
    engine: GosubEngine,
    #[allow(dead_code)]
    zone: Zone,
    tab: TabHandle,
    tab_id: TabId,
    compositor: Arc<RwLock<DefaultCompositor>>,
    #[allow(dead_code)]
    proxy: EventLoopProxy<()>,

    window: Option<Arc<Window>>,
    sb_surface: Option<Surface<Arc<Window>, Arc<Window>>>,
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
        let Some(sb_surface) = &mut self.sb_surface else { return };
        let (win_w, win_h) = self.surface_size;
        if win_w == 0 || win_h == 0 {
            return;
        }

        let Ok(nw) = NonZeroU32::try_from(win_w) else { return };
        let Ok(nh) = NonZeroU32::try_from(win_h) else { return };
        if sb_surface.resize(nw, nh).is_err() {
            return;
        }

        let Ok(mut buf) = sb_surface.buffer_mut() else { return };
        buf.fill(0x00FF_FFFF);

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
                );
            }
        }

        draw_address_bar(&mut buf, win_w, &self.url_input, self.addr_focused);
        buf.present().unwrap_or_default();
    }

    fn is_in_addr_bar(&self, y: f64) -> bool {
        y < ADDRESS_BAR_HEIGHT as f64
    }
    fn css_x(&self, x: f64) -> f32 {
        (x + self.scroll.0 as f64) as f32
    }
    fn css_y(&self, y: f64) -> f32 {
        (y - ADDRESS_BAR_HEIGHT as f64 + self.scroll.1 as f64) as f32
    }
}

impl ApplicationHandler<()> for BrowserApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let attrs = WindowAttributes::default()
            .with_title("Gosub Browser — winit + Skia GPU")
            .with_inner_size(LogicalSize::new(1024u32, 768u32));

        let window = Arc::new(event_loop.create_window(attrs).expect("create window"));
        let ctx = softbuffer::Context::new(window.clone()).expect("softbuffer context");
        let sb_surface = Surface::new(&ctx, window.clone()).expect("softbuffer surface");

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
        self.sb_surface = Some(sb_surface);
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
                if !self.is_in_addr_bar(position.y) {
                    let (x, y) = (self.css_x(position.x), self.css_y(position.y));
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
                if self.is_in_addr_bar(self.cursor.y) {
                    self.addr_focused = true;
                    if let Some(w) = &self.window {
                        w.request_redraw();
                    }
                } else {
                    self.addr_focused = false;
                    let (x, y) = (self.css_x(self.cursor.x), self.css_y(self.cursor.y));
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
) {
    match handle {
        ExternalHandle::CpuPixelsOwned {
            width,
            height,
            stride,
            pixels,
            ..
        } => {
            for row in 0..height.min(content_h) as usize {
                for col in 0..(width as usize).min(win_w as usize) {
                    let off = row * stride as usize + col * 4;
                    let b = pixels[off] as u32;
                    let g = pixels[off + 1] as u32;
                    let r = pixels[off + 2] as u32;
                    let idx = (addr_h as usize + row) * win_w as usize + col;
                    if idx < buf.len() {
                        buf[idx] = (r << 16) | (g << 8) | b;
                    }
                }
            }
        }
        ExternalHandle::TileCache {
            tiles,
            page_height: ph,
            scroll_x,
            scroll_y,
            dpr,
            ..
        } => {
            *page_height = ph;
            let d = dpr as usize;
            let (sx, sy) = (scroll_x as usize * d, scroll_y as usize * d);
            for tile in tiles.iter() {
                let screen_x = (tile.page_x as usize * d).saturating_sub(sx);
                let screen_y = (tile.page_y as usize * d).saturating_sub(sy);
                let (tw, th) = (tile.width as usize, tile.height as usize);
                let tile_u32 =
                    unsafe { std::slice::from_raw_parts(tile.data.as_ptr() as *const u32, tile.data.len() / 4) };
                for row in 0..th {
                    let dst_y = addr_h as usize + screen_y + row;
                    if dst_y >= (addr_h + content_h) as usize {
                        break;
                    }
                    let cw = tw.min(win_w as usize - screen_x.min(win_w as usize));
                    if cw == 0 {
                        break;
                    }
                    for col in 0..cw {
                        let px = tile_u32[row * tw + col];
                        let idx = dst_y * win_w as usize + screen_x + col;
                        if idx < buf.len() {
                            buf[idx] = ((px >> 16) & 0xFF) << 16 | ((px >> 8) & 0xFF) << 8 | (px & 0xFF);
                        }
                    }
                }
            }
        }
        _ => {}
    }
}

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
    let mut bg = Paint::new(
        if focused {
            Color4f::new(1.0, 1.0, 1.0, 1.0)
        } else {
            Color4f::new(0.97, 0.97, 0.97, 1.0)
        },
        None,
    );
    bg.set_anti_alias(true);
    canvas.draw_rect(SkRect::new(4.0, 5.0, (w - 4) as f32, (h - 5) as f32), &bg);
    let border = if focused {
        Color4f::new(0.26, 0.52, 0.96, 1.0)
    } else {
        Color4f::new(0.7, 0.7, 0.7, 1.0)
    };
    let mut bp = Paint::new(border, None);
    bp.set_anti_alias(true);
    bp.set_style(skia_safe::PaintStyle::Stroke);
    bp.set_stroke_width(1.0);
    canvas.draw_rect(SkRect::new(4.5, 5.5, (w - 5) as f32, (h - 5) as f32), &bp);
    let typeface = FontMgr::new()
        .legacy_make_typeface(None, FontStyle::normal())
        .unwrap_or_else(|| {
            FontMgr::new()
                .legacy_make_typeface("sans-serif", FontStyle::normal())
                .expect("no typeface")
        });
    let font = Font::new(typeface, 14.0);
    let mut tp = Paint::new(Color4f::new(0.0, 0.0, 0.0, 1.0), None);
    tp.set_anti_alias(true);
    canvas.draw_str(url, (10.0f32, h as f32 - 10.0), &font, &tp);
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
    simple_logger::SimpleLogger::new().with_level(log::LevelFilter::Warn).env().init().unwrap_or_default();

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

    // Build a window + GL config via glutin-winit.
    let attrs = WindowAttributes::default()
        .with_title("Gosub Browser — winit + Skia GPU")
        .with_inner_size(LogicalSize::new(1024u32, 768u32));

    let (gl_window, gl_config) = DisplayBuilder::new()
        .with_window_attributes(Some(attrs))
        .build(&event_loop, glutin::config::ConfigTemplateBuilder::new(), |cfgs| {
            cfgs.reduce(|a, b| if b.num_samples() > a.num_samples() { b } else { a })
                .expect("no suitable GL config")
        })
        .expect("display build failed");

    let gl_window = gl_window.expect("window");
    let gl_display = gl_config.display();

    // Create the GL context.
    let context_attrs = ContextAttributesBuilder::new()
        .with_context_api(ContextApi::OpenGl(None))
        .build(gl_window.window_handle().ok().map(|h| h.as_raw()));

    let not_current = unsafe {
        gl_display
            .create_context(&gl_config, &context_attrs)
            .expect("GL context")
    };

    // Create the window surface and make the context current.
    let surface_attrs = gl_window
        .build_surface_attributes(Default::default())
        .expect("surface attrs");
    let gl_surface = unsafe {
        gl_display
            .create_window_surface(&gl_config, &surface_attrs)
            .expect("GL surface")
    };
    let gl_context = not_current.make_current(&gl_surface).expect("make current");

    let glutin_ctx = Arc::new(GlutinContext {
        gl_config,
        context: gl_context,
        gl_surface,
    });

    let compositor = Arc::new(RwLock::new(DefaultCompositor::new({
        let p = proxy.clone();
        move || {
            let _ = p.send_event(());
        }
    })));

    let backend = SkiaGpuBackend::new(glutin_ctx).expect("SkiaGpuBackend");
    let mut engine = GosubEngine::new(None, Arc::new(backend), compositor.clone());
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

    // gl_window is dropped here; the ApplicationHandler creates the softbuffer window.

    let mut app = BrowserApp {
        engine,
        zone,
        tab,
        tab_id,
        compositor,
        proxy,
        window: None,
        sb_surface: None,
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
