//! Minimal browser window: Skia GPU (OpenGL/Ganesh) compositor + winit.
//!
//! Usage: cargo run -p example-winit-skia-gpu -- https://example.com
//!
//! The engine rasterizes tiles on worker threads using SkiaRasterizer (CPU).
//! The main (event-loop) thread receives a TileCache and composites the tiles
//! directly onto the GL window surface via Skia's Ganesh GPU backend - no CPU
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
use gosub_engine::DefaultRenderConfig;
use gosub_engine::GosubEngine;
use gosub_renderer_skia::{SkiaBackend, SkiaFontSystem};

type AppConfig = DefaultRenderConfig<SkiaBackend, SkiaFontSystem>;
use gosub_render_pipeline::render::backend::{anchored_tile_pos, CachedTile, ExternalHandle};
use gosub_render_pipeline::render::DefaultCompositor;
use once_cell::sync::Lazy;
use skia_safe::gpu::ganesh::surface_ganesh;
use skia_safe::gpu::{self, gl::FramebufferInfo, DirectContext, SurfaceOrigin};
use skia_safe::{ColorType, ImageInfo, Rect as SkRect, Surface};
use std::ffi::CString;
use std::num::NonZeroU32;
use std::sync::Arc;
use tokio::runtime::{Builder, Runtime};
use uuid::uuid;
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, MouseButton as WinitMouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy};
use winit::raw_window_handle::HasWindowHandle;
use winit::window::{Window, WindowAttributes, WindowId};

const DEFAULT_ZONE: uuid::Uuid = uuid!("f1234567-abcd-4000-8000-00000000000d");
const SCROLL_MULTIPLIER: f32 = 134.0;

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
    engine: GosubEngine<AppConfig>,
    #[allow(dead_code)]
    zone: Zone<AppConfig>,
    tab: TabHandle,
    tab_id: TabId,
    compositor: Arc<DefaultCompositor>,
    #[allow(dead_code)]
    proxy: EventLoopProxy<()>,

    window: Option<Arc<Window>>,
    gl: Option<GlState>,
    surface_size: (u32, u32),

    cursor: PhysicalPosition<f64>,
    scroll: (f32, f32),
    page_height: f32,
    viewport: (u32, u32),
}

impl BrowserApp {
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
        if let Some(handle) = self.compositor.frame_for(self.tab_id) {
            composite_tiles(canvas, win_w, 0.0, win_h, &handle, &mut self.page_height);
        }

        drop(skia_surface);
        gl.flush();
    }

    fn css_x(&self, x: f64) -> f32 {
        (x + self.scroll.0 as f64) as f32
    }
    fn css_y(&self, y: f64) -> f32 {
        (y + self.scroll.1 as f64) as f32
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
                let content_h = height;
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
                let (x, y) = (self.css_x(position.x), self.css_y(position.y));
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

            // Release always reaches the engine so drags end.
            WindowEvent::MouseInput {
                state: ElementState::Released,
                button: WinitMouseButton::Left,
                ..
            } => {
                let (x, y) = (self.css_x(self.cursor.x), self.css_y(self.cursor.y));
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
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }

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
        // anchored_tile_pos handles scroll / fixed / sticky uniformly from the engine's scroll.
        let (vx, vy) = anchored_tile_pos(
            tile.page_x as f64,
            tile.page_y as f64,
            *sx as f64,
            *sy as f64,
            tile.anchor,
        );
        let screen_x = vx as f32;
        let screen_y = vy as f32 + addr_h;

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
        // Fade the whole tile by its layer's group opacity (e.g. a translucent fixed navbar).
        let paint = if tile.opacity < 1.0 {
            let mut p = skia_safe::Paint::default();
            p.set_alpha_f(tile.opacity);
            Some(p)
        } else {
            None
        };
        canvas.draw_image(&image, (x, y), paint.as_ref());
    }
}

// ── Address bar (drawn via Skia on the GPU canvas) ────────────────────────────

// ── main ──────────────────────────────────────────────────────────────────────

fn main() {
    eprintln!(
        "{} v{} — winit browser window, Skia GPU (OpenGL) rendering",
        env!("CARGO_BIN_NAME"),
        env!("CARGO_PKG_VERSION")
    );

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
    let compositor = Arc::new(DefaultCompositor::new({
        let p = proxy.clone();
        move || {
            let _ = p.send_event(());
        }
    }));

    // Rasterize tiles on the CPU via the Skia backend; this example then uploads those tiles and
    // composites them onto the GL window surface through Skia's Ganesh GPU backend. (A NullBackend
    // produces no tiles - the engine needs a real rasterizer to emit TileCache frames.)
    let backend = SkiaBackend::new();
    let mut engine = GosubEngine::<AppConfig>::new(None, Arc::new(backend), compositor.clone());
    // GOSUB_COLOR_SCHEME=dark renders pages and native controls in the dark scheme.
    if std::env::var("GOSUB_COLOR_SCHEME").is_ok_and(|v| v.eq_ignore_ascii_case("dark")) {
        let _ = engine.settings().set(
            "renderer.color_scheme",
            gosub_config::settings::Setting::String("dark".into()),
        );
    }
    let _engine_task = TOKIO_RT.spawn(engine.start().expect("engine start"));

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
    let content_h = size.height;
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
        cursor: PhysicalPosition::default(),
        scroll: (0.0, 0.0),
        page_height: 0.0,
        viewport: (size.width, content_h),
    };

    event_loop.run_app(&mut app).expect("event loop");
}
