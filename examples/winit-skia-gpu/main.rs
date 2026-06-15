//! Minimal browser window: Skia GPU (OpenGL/Ganesh) compositor + winit.
//!
//! Usage: cargo run -p example-winit-skia-gpu -- https://example.com
//!
//! The engine rasterizes tiles on worker threads using SkiaRasterizer (CPU).
//! The main (event-loop) thread receives a TileCache and composites the tiles
//! directly onto the GL window surface via Skia's Ganesh GPU backend — no CPU
//! readback required.

#[link(name = "GL")]
extern "C" {}

use glutin::config::{Config, GlConfig};
use glutin::context::{ContextApi, ContextAttributesBuilder, PossiblyCurrentContext};
use glutin::display::GetGlDisplay;
use glutin::prelude::{GlDisplay, GlSurface, NotCurrentGlContext as _};
use glutin::surface::{Surface as GlSurface_, WindowSurface};
use glutin_winit::{DisplayBuilder, GlWindow};
use gosub_engine::events::{EngineEvent, MouseButton, NavigationEvent, TabCommand};
use gosub_engine::storage::{InMemorySessionStore, PartitionPolicy, SqliteLocalStore, StorageService};
use gosub_engine::tab::{TabDefaults, TabHandle, TabId};
use gosub_engine::zone::{Zone, ZoneConfig, ZoneId, ZoneServices};
use gosub_engine::GosubEngine;
use gosub_render_pipeline::render::backend::{CachedTile, ExternalHandle};
use gosub_render_pipeline::render::DefaultCompositor;
use once_cell::sync::Lazy;
use parking_lot::RwLock;
use skia_safe::gpu::ganesh::surface_ganesh;
use skia_safe::gpu::{self, gl::FramebufferInfo, DirectContext, SurfaceOrigin};
use skia_safe::{Color4f, ColorType, Font, FontMgr, FontStyle, ImageInfo, Paint, Rect as SkRect, Surface};
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

const DEFAULT_ZONE: uuid::Uuid = uuid!("f1234567-abcd-4000-8000-00000000000d");
const ADDRESS_BAR_HEIGHT: f32 = 36.0;
const SCROLL_MULTIPLIER: f32 = 12.5;

static TOKIO_RT: Lazy<Runtime> = Lazy::new(|| {
    Builder::new_multi_thread()
        .enable_io()
        .enable_time()
        .thread_name("gosub-winit-skia-gpu-rt")
        .build()
        .expect("tokio runtime")
});

// ── GL state kept on the main thread ─────────────────────────────────────────

#[allow(dead_code)]
struct GlState {
    gl_context: PossiblyCurrentContext,
    gl_surface: GlSurface_<WindowSurface>,
    gl_config: Config,
    direct_context: DirectContext,
}

impl GlState {
    /// Create a Skia GPU surface that wraps the current GL default framebuffer.
    fn skia_surface(&mut self, width: i32, height: i32) -> Option<Surface> {
        let fb_info = FramebufferInfo {
            fboid: 0,
            format: skia_safe::gpu::gl::Format::RGBA8.into(),
            ..Default::default()
        };
        let render_target = gpu::backend_render_targets::make_gl((width, height), None, 8, fb_info);
        surface_ganesh::wrap_backend_render_target(
            &mut self.direct_context,
            &render_target,
            SurfaceOrigin::BottomLeft,
            ColorType::RGBA8888,
            None,
            None,
        )
    }

    fn flush(&mut self) {
        self.direct_context.flush_and_submit();
        self.gl_surface.swap_buffers(&self.gl_context).unwrap_or_default();
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
    gl: Option<GlState>,
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
        if !s.contains("://") {
            s = format!("https://{s}");
            self.url_input = s.clone();
        }
        let Ok(_) = Url::parse(&s) else { return };
        self.scroll = (0.0, 0.0);
        let tab = self.tab.clone();
        TOKIO_RT.spawn(async move {
            let _ = tab.send(TabCommand::Navigate { url: s }).await;
            let _ = tab.send(TabCommand::ResumeDrawing { fps: 30 }).await;
        });
    }

    fn redraw(&mut self) {
        let Some(gl) = self.gl.as_mut() else { return };
        let (win_w, win_h) = self.surface_size;
        if win_w == 0 || win_h == 0 {
            return;
        }

        let Some(mut skia_surface) = gl.skia_surface(win_w as i32, win_h as i32) else {
            return;
        };
        let canvas = skia_surface.canvas();

        // White background
        canvas.clear(skia_safe::Color4f::new(1.0, 1.0, 1.0, 1.0));

        // Composite page tiles
        let content_h = win_h.saturating_sub(ADDRESS_BAR_HEIGHT as u32);
        {
            let guard = self.compositor.read();
            if let Some(handle) = guard.frame_for(self.tab_id) {
                composite_tiles(
                    canvas,
                    win_w,
                    ADDRESS_BAR_HEIGHT,
                    content_h,
                    &handle,
                    &mut self.page_height,
                );
            }
        }

        // Address bar (drawn on top via Skia)
        draw_address_bar(
            canvas,
            win_w,
            ADDRESS_BAR_HEIGHT as i32,
            &self.url_input,
            self.addr_focused,
        );

        drop(skia_surface);
        gl.flush();
    }

    fn is_addr_bar(&self, y: f64) -> bool {
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
    fn resumed(&mut self, _event_loop: &ActiveEventLoop) {
        // Window and GL context are created before the event loop starts on desktop.
        // Nothing to do here.
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
                let content_h = height.saturating_sub(ADDRESS_BAR_HEIGHT as u32);
                self.viewport = (width, content_h);
                self.scroll = (0.0, 0.0);
                if let Some(gl) = &self.gl {
                    gl.gl_surface.resize(
                        &gl.gl_context,
                        NonZeroU32::new(width).unwrap(),
                        NonZeroU32::new(height).unwrap(),
                    );
                }
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
                if !self.is_addr_bar(position.y) {
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
                if self.is_addr_bar(self.cursor.y) {
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

// ── GPU tile compositing ───────────────────────────────────────────────────────

fn composite_tiles(
    canvas: &skia_safe::Canvas,
    win_w: u32,
    addr_h: f32,
    content_h: u32,
    handle: &ExternalHandle,
    page_height: &mut f32,
) {
    let ExternalHandle::TileCache {
        tiles,
        page_height: ph,
        scroll_x: sx,
        scroll_y: sy,
        ..
    } = handle
    else {
        return;
    };
    *page_height = *ph;

    canvas.save();
    canvas.clip_rect(
        SkRect::from_xywh(0.0, addr_h, win_w as f32, content_h as f32),
        None,
        None,
    );

    for tile in tiles.iter() {
        // screen position = page position minus the scroll offset embedded in the handle,
        // consistent with winit-skia and winit-cairo.
        let screen_x = tile.page_x - sx;
        let screen_y = tile.page_y - sy + addr_h;

        // Cull tiles outside the viewport
        if screen_x + tile.width as f32 <= 0.0 {
            continue;
        }
        if screen_y + tile.height as f32 <= addr_h {
            continue;
        }
        if screen_x >= win_w as f32 {
            continue;
        }
        if screen_y >= addr_h + content_h as f32 {
            continue;
        }

        blit_tile(canvas, tile, screen_x, screen_y);
    }

    canvas.restore();
}

fn blit_tile(canvas: &skia_safe::Canvas, tile: &CachedTile, x: f32, y: f32) {
    let info = ImageInfo::new(
        (tile.width as i32, tile.height as i32),
        skia_safe::ColorType::BGRA8888,
        skia_safe::AlphaType::Premul,
        None,
    );
    if let Some(image) =
        skia_safe::images::raster_from_data(&info, skia_safe::Data::new_copy(&tile.data), (tile.width * 4) as usize)
    {
        canvas.draw_image(&image, (x, y), None);
    }
}

// ── Address bar (drawn via Skia on the GPU canvas) ────────────────────────────

fn draw_address_bar(canvas: &skia_safe::Canvas, win_w: u32, h: i32, url: &str, focused: bool) {
    let w = win_w as f32;
    let hf = h as f32;

    let bg = if focused {
        Color4f::new(0.98, 0.98, 0.98, 1.0)
    } else {
        Color4f::new(0.93, 0.93, 0.93, 1.0)
    };
    let mut paint = Paint::new(bg, None);
    canvas.draw_rect(SkRect::from_xywh(0.0, 0.0, w, hf), &paint);

    let field_bg = if focused {
        Color4f::new(1.0, 1.0, 1.0, 1.0)
    } else {
        Color4f::new(0.97, 0.97, 0.97, 1.0)
    };
    paint.set_color4f(field_bg, None);
    paint.set_anti_alias(true);
    canvas.draw_round_rect(SkRect::from_xywh(6.0, 5.0, w - 12.0, hf - 10.0), 4.0, 4.0, &paint);

    let border = if focused {
        Color4f::new(0.26, 0.52, 0.96, 1.0)
    } else {
        Color4f::new(0.7, 0.7, 0.7, 1.0)
    };
    paint.set_color4f(border, None);
    paint.set_style(skia_safe::PaintStyle::Stroke);
    paint.set_stroke_width(1.0);
    canvas.draw_round_rect(SkRect::from_xywh(6.5, 5.5, w - 13.0, hf - 11.0), 4.0, 4.0, &paint);

    thread_local! { static FONT_MGR: FontMgr = FontMgr::new(); }
    let typeface = FONT_MGR.with(|fm| {
        fm.legacy_make_typeface(None, FontStyle::normal()).unwrap_or_else(|| {
            fm.legacy_make_typeface("sans-serif", FontStyle::normal())
                .expect("typeface")
        })
    });
    let font = Font::new(typeface, 14.0);
    paint.set_color4f(Color4f::new(0.0, 0.0, 0.0, 1.0), None);
    paint.set_style(skia_safe::PaintStyle::Fill);
    canvas.draw_str(url, (12.0f32, hf - 10.0), &font, &paint);
}

// ── main ──────────────────────────────────────────────────────────────────────

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

    // Create window + GL config.
    let win_attrs = WindowAttributes::default()
        .with_title("Gosub Browser — winit + Skia GPU")
        .with_inner_size(LogicalSize::new(1024u32, 768u32));

    let (gl_window, gl_config) = DisplayBuilder::new()
        .with_window_attributes(Some(win_attrs))
        .build(&event_loop, glutin::config::ConfigTemplateBuilder::new(), |cfgs| {
            cfgs.reduce(|a, b| if b.num_samples() > a.num_samples() { b } else { a })
                .expect("no GL config")
        })
        .expect("display build");

    let gl_window = gl_window.expect("window");
    let gl_display = gl_config.display();

    let ctx_attrs = ContextAttributesBuilder::new()
        .with_context_api(ContextApi::OpenGl(None))
        .build(gl_window.window_handle().ok().map(|h| h.as_raw()));

    let not_current = unsafe { gl_display.create_context(&gl_config, &ctx_attrs).expect("GL context") };

    let surf_attrs = gl_window
        .build_surface_attributes(Default::default())
        .expect("surface attrs");
    let gl_surface = unsafe {
        gl_display
            .create_window_surface(&gl_config, &surf_attrs)
            .expect("GL surface")
    };
    let gl_context = not_current.make_current(&gl_surface).expect("make current");

    // Build Skia DirectContext using the GL interface.
    let interface = skia_safe::gpu::gl::Interface::new_load_with(|name| {
        let c = CString::new(name).unwrap_or_default();
        gl_display.get_proc_address(&c)
    })
    .expect("GL interface");
    let direct_context = skia_safe::gpu::direct_contexts::make_gl(interface, None).expect("Skia DirectContext");

    let gl_state = GlState {
        gl_context,
        gl_surface,
        gl_config,
        direct_context,
    };

    // Engine + compositor
    let compositor = Arc::new(RwLock::new(DefaultCompositor::new({
        let p = proxy.clone();
        move || {
            let _ = p.send_event(());
        }
    })));

    // Use a null render backend — TileCache frames are submitted directly by the engine
    // without going through a render backend's display-list pipeline.
    let backend = gosub_render_pipeline::render::backends::null::NullBackend::new();
    let mut engine: GosubEngine = GosubEngine::new(None, Arc::new(backend), compositor.clone());
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
        .expect("zone");
    let tab = TOKIO_RT
        .block_on(zone.create_tab(
            TabDefaults {
                url: None,
                title: Some("Gosub".to_string()),
                viewport: None,
            },
            None,
        ))
        .expect("tab");

    let tab_id = tab.tab_id;
    let nav_tab = tab.clone();
    let nav_url = initial_url.clone();
    TOKIO_RT.spawn(async move {
        let _ = nav_tab.send(TabCommand::Navigate { url: nav_url }).await;
    });

    let size = gl_window.inner_size();
    let content_h = size.height.saturating_sub(ADDRESS_BAR_HEIGHT as u32);
    {
        let t = tab.clone();
        TOKIO_RT.block_on(async move {
            let _ = t
                .send(TabCommand::SetViewport {
                    x: 0,
                    y: 0,
                    width: size.width,
                    height: content_h,
                })
                .await;
            let _ = t.send(TabCommand::ResumeDrawing { fps: 30 }).await;
        });
    }

    let window = Arc::new(gl_window);

    let mut app = BrowserApp {
        engine,
        zone,
        tab,
        tab_id,
        compositor,
        proxy,
        window: Some(window),
        gl: Some(gl_state),
        surface_size: (size.width, size.height),
        url_input: initial_url,
        addr_focused: false,
        cursor: PhysicalPosition::default(),
        scroll: (0.0, 0.0),
        page_height: 0.0,
        viewport: (size.width, content_h),
    };

    event_loop.run_app(&mut app).expect("event loop");
}
