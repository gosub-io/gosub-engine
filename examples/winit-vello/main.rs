//! Minimal browser window: Vello (GPU) rasterizer + winit toolkit.
//!
//! Usage: cargo run --example winit-vello -- https://example.com
//!
//! Press Ctrl+L to focus the address bar (URL shown in window title while typing).
//! No GTK/Cairo dependency — pure winit + wgpu.
//!
//! Architecture note: the wgpu adapter and device are created inside `resumed()`
//! after the window exists, so the adapter can be selected for surface compatibility.
//! On Wayland an incompatible adapter causes `get_current_texture()` to silently fail
//! every frame, keeping the surface un-committed and the window invisible.

use gosub_engine::events::{EngineEvent, MouseButton, NavigationEvent, TabCommand};
use gosub_engine::storage::{InMemorySessionStore, PartitionPolicy, SqliteLocalStore, StorageService};
use gosub_engine::tab::{TabDefaults, TabHandle, TabId};
use gosub_engine::zone::{Zone, ZoneConfig, ZoneId, ZoneServices};
use gosub_engine::DefaultRenderConfig;
use gosub_engine::GosubEngine;
use gosub_render_pipeline::render::backend::ExternalHandle;
use gosub_render_pipeline::render::{argb_u32_to_rgba8, composite_tiles, DefaultCompositor, TileTarget, Viewport};
use gosub_renderer_vello::{VelloBackend, WgpuContextProvider};
use gosub_winit::{GpuPresenter, WinitWgpuContextProvider};
use once_cell::sync::Lazy;
use std::sync::Arc;
use tokio::runtime::{Builder, Runtime};
use url::Url;
use uuid::uuid;
use vello::wgpu;
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, KeyEvent, MouseButton as WinitMouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::{WindowAttributes, WindowId};

const DEFAULT_ZONE: uuid::Uuid = uuid!("f1234567-abcd-4000-8000-000000000007");
/// CSS pixels scrolled per wheel notch. winit delivers a mouse wheel as `LineDelta(0, ±1)` (1.0
/// per notch), so this is the per-tick distance directly. Calibrated to Firefox's ~134 CSS px/tick
/// (measured, and constant across zoom). Trackpad `PixelDelta` is handled separately, unscaled.
const SCROLL_MULTIPLIER: f32 = 134.0;

type AppConfig = DefaultRenderConfig<VelloBackend<WinitWgpuContextProvider>>;

static TOKIO_RT: Lazy<Runtime> = Lazy::new(|| {
    Builder::new_multi_thread()
        .enable_io()
        .enable_time()
        .thread_name("gosub-winit-vello-rt")
        .build()
        .expect("tokio runtime")
});

// ── Lazy GPU + engine state (created in resumed()) ────────────────────────────

struct RuntimeState {
    context: Arc<WinitWgpuContextProvider>,
    gpu: GpuPresenter,
    // These fields keep background tasks alive for the process lifetime.
    #[allow(dead_code)]
    engine: GosubEngine<AppConfig>,
    #[allow(dead_code)]
    zone: Zone<AppConfig>,
    tab: TabHandle,
    tab_id: TabId,
}

// ── Application ───────────────────────────────────────────────────────────────

struct BrowserApp {
    // Available from the start (before any window exists).
    instance: wgpu::Instance,
    compositor: Arc<DefaultCompositor>,
    proxy: EventLoopProxy<()>,
    initial_url: String,

    // Populated in resumed() once a surface-compatible device is obtained.
    state: Option<RuntimeState>,

    // UI state
    url_input: String,
    addr_focused: bool,
    current_url: String,
    modifiers: ModifiersState,
    /// Cursor position in physical pixels, as winit reports it.
    cursor: PhysicalPosition<f64>,
    /// Engine viewport in *logical* (CSS) pixels — physical window size ÷ `scale`.
    viewport: (u32, u32),
    /// Display scale factor (physical ÷ logical px). The wgpu surface stays at physical size;
    /// the engine lays out and paints in logical px, and the full-screen blit upscales the
    /// resulting texture to the physical surface. Keeps page content the same on-screen size as
    /// DPR-aware browsers on fractionally scaled displays instead of rendering everything smaller.
    scale: f64,
}

impl BrowserApp {
    fn navigate(&mut self) {
        let Some(rt) = &self.state else { return };
        let tab = rt.tab.clone();
        let mut s = self.url_input.clone();
        if !s.starts_with("http://") && !s.starts_with("https://") {
            s = format!("https://{s}");
        }
        let Ok(_) = Url::parse(&s) else { return };
        self.url_input = s.clone();
        self.addr_focused = false;
        self.update_title();
        TOKIO_RT.spawn(async move {
            let _ = tab.send(TabCommand::Navigate { url: s }).await;
            // 60fps so the per-frame smooth-scroll deltas render as a smooth glide, not ~5 steps.
            let _ = tab.send(TabCommand::ResumeDrawing { fps: 60 }).await;
        });
    }

    fn update_title(&self) {
        let Some(rt) = &self.state else { return };
        let title = if self.addr_focused {
            format!("URL: {} — Gosub (Enter to navigate, Esc to cancel)", self.url_input)
        } else {
            format!("Gosub Browser — {}", self.current_url)
        };
        rt.gpu.window().set_title(&title);
    }

    /// Convert a physical pixel length to logical (CSS) pixels for the engine.
    fn to_logical(&self, physical: u32) -> u32 {
        ((physical as f64 / self.scale).round() as u32).max(1)
    }

    /// Convert a physical cursor coordinate to logical (CSS) pixels for the engine.
    fn cursor_logical(&self, physical: f64) -> f32 {
        (physical / self.scale) as f32
    }

    fn redraw(&mut self) {
        let Some(tab_id) = self.state.as_ref().map(|rt| rt.tab_id) else {
            return;
        };
        let Some(handle) = self.compositor.frame_for(tab_id) else {
            return;
        };

        match handle {
            // GPU path — the engine renders the whole display list into a single GPU
            // texture (Vello's `raster_strategy() == None`) and hands us its id. Blit it
            // straight to the swap chain, no CPU round-trip.
            ExternalHandle::WgpuTextureId { id, .. } => {
                let rt = self.state.as_ref().unwrap();
                if let Some((_, view)) = rt.context.get_texture(id) {
                    rt.gpu.present(&view);
                }
            }

            // CPU tile path — fallback for tile-rasterizing backends (Cairo/Skia). Composite
            // the visible tiles into an RGBA buffer, then upload + blit it via wgpu.
            ExternalHandle::TileCache {
                tiles,
                dpr,
                viewport_width,
                viewport_height,
                scroll_x,
                scroll_y,
                ..
            } => {
                let w = (viewport_width * dpr) as usize;
                let h = (viewport_height * dpr) as usize;
                if w == 0 || h == 0 {
                    return;
                }
                // Composite at the engine's authoritative (animated) scroll, carried on the handle,
                // onto an opaque-white premultiplied background, then convert to RGBA8 for wgpu.
                let mut buf = vec![0xFFFF_FFFFu32; w * h];
                composite_tiles(
                    &tiles,
                    dpr,
                    (scroll_x, scroll_y),
                    &mut TileTarget {
                        buf: &mut buf,
                        stride: w,
                        origin_x: 0,
                        origin_y: 0,
                        width: w,
                        height: h,
                    },
                );
                let rgba = argb_u32_to_rgba8(&buf);

                let rt = self.state.as_ref().unwrap();
                rt.gpu.present_rgba(&rgba, w as u32, h as u32);
            }

            _ => {}
        }
    }
}

impl ApplicationHandler<()> for BrowserApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }

        // ── 1. Create window ──────────────────────────────────────────────────
        let attrs = WindowAttributes::default()
            .with_title("Gosub Browser — winit + Vello")
            .with_inner_size(LogicalSize::new(1024u32, 768u32));
        let window = Arc::new(event_loop.create_window(attrs).expect("create window"));

        // ── 2. GPU presenter: surface + surface-compatible adapter/device + blit pipeline ──
        // (gosub_winit handles the Wayland surface-hint and non-sRGB swap-chain selection.)
        let gpu = match TOKIO_RT.block_on(GpuPresenter::new(&self.instance, window)) {
            Ok(g) => g,
            Err(e) => {
                log::error!("gpu init: {e}");
                return;
            }
        };
        let size = gpu.window().inner_size();

        // ── 3. Build VelloBackend and engine ──────────────────────────────────
        let context = Arc::new(WinitWgpuContextProvider::new(gpu.device().clone(), gpu.queue().clone()));
        let backend = match VelloBackend::new(context.clone()) {
            Ok(b) => b,
            Err(e) => {
                log::error!("VelloBackend: {e}");
                return;
            }
        };

        let mut engine = GosubEngine::<DefaultRenderConfig<_>>::new(None, Arc::new(backend), self.compositor.clone());
        let _engine_task = TOKIO_RT.spawn(engine.start().expect("engine start"));

        // Forward navigation events → proxy → request_redraw.
        let proxy_ev = self.proxy.clone();
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

        // ── 6. Zone + tab ─────────────────────────────────────────────────────
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

        // The wgpu surface (configured above) stays at physical `size`; the engine works in
        // logical (CSS) px so content matches DPR-aware browsers on fractionally scaled displays.
        self.scale = gpu.window().scale_factor();
        eprintln!(
            "DPR_DEBUG scale={} physical={}x{} logical={}x{}",
            self.scale,
            size.width,
            size.height,
            self.to_logical(size.width),
            self.to_logical(size.height)
        );
        let logical_w = self.to_logical(size.width);
        let logical_h = self.to_logical(size.height);

        let tab = TOKIO_RT
            .block_on(zone.create_tab(
                TabDefaults {
                    url: None,
                    title: Some("Gosub".to_string()),
                    viewport: Some(Viewport::new(0, 0, logical_w, logical_h)),
                },
                None,
            ))
            .expect("create_tab");

        let tab_id = tab.tab_id;
        self.viewport = (logical_w, logical_h);

        // Navigate + start drawing.
        let nav_tab = tab.clone();
        let nav_url = self.initial_url.clone();
        TOKIO_RT.spawn(async move {
            let _ = nav_tab.send(TabCommand::Navigate { url: nav_url }).await;
            // 60fps so the per-frame smooth-scroll deltas render as a smooth glide, not ~5 steps.
            let _ = nav_tab.send(TabCommand::ResumeDrawing { fps: 60 }).await;
        });

        self.state = Some(RuntimeState {
            context,
            gpu,
            engine,
            zone,
            tab,
            tab_id,
        });

        self.update_title();
    }

    fn user_event(&mut self, _: &ActiveEventLoop, _: ()) {
        if let Some(rt) = &self.state {
            rt.gpu.window().request_redraw();
        }
    }

    /// Drive a steady present cadence so engine-side updates (window resize, scroll, hover,
    /// animations) appear live, the way the GTK example's 16ms `queue_draw` timer does.
    ///
    /// Without this the window only repainted when a navigation event woke the proxy, so a
    /// resize re-laid-out the page in the engine but never presented the new frame. Re-arming a
    /// redraw every ~16ms (capped via `WaitUntil`, so the loop still sleeps rather than busy-
    /// spinning) keeps the swap chain showing the latest compositor frame at ~60fps.
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(rt) = &self.state {
            rt.gpu.window().request_redraw();
        }
        event_loop.set_control_flow(ControlFlow::WaitUntil(
            std::time::Instant::now() + std::time::Duration::from_millis(16),
        ));
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),

            WindowEvent::RedrawRequested => self.redraw(),

            WindowEvent::Resized(PhysicalSize { width, height }) => {
                if width == 0 || height == 0 {
                    return;
                }
                // Surface follows the physical framebuffer; the engine viewport is logical.
                let (lw, lh) = (self.to_logical(width), self.to_logical(height));
                if let Some(rt) = &mut self.state {
                    rt.gpu.resize(width, height);
                    let tab = rt.tab.clone();
                    TOKIO_RT.spawn(async move {
                        let _ = tab
                            .send(TabCommand::SetViewport {
                                x: 0,
                                y: 0,
                                width: lw,
                                height: lh,
                            })
                            .await;
                    });
                }
                self.viewport = (lw, lh);
            }

            // The window moved to a display with a different scale (e.g. dragged between
            // monitors). Re-derive logical size from the new scale and re-send the viewport.
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                self.scale = scale_factor;
                if let Some(rt) = &self.state {
                    let size = rt.gpu.window().inner_size();
                    let (lw, lh) = (self.to_logical(size.width), self.to_logical(size.height));
                    self.viewport = (lw, lh);
                    let tab = rt.tab.clone();
                    TOKIO_RT.spawn(async move {
                        let _ = tab
                            .send(TabCommand::SetViewport {
                                x: 0,
                                y: 0,
                                width: lw,
                                height: lh,
                            })
                            .await;
                    });
                }
            }

            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = position;
                if let Some(rt) = &self.state {
                    let x = self.cursor_logical(position.x);
                    let y = self.cursor_logical(position.y);
                    let tab = rt.tab.clone();
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
                if let Some(rt) = &self.state {
                    let x = self.cursor_logical(self.cursor.x);
                    let y = self.cursor_logical(self.cursor.y);
                    let tab = rt.tab.clone();
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
                    // Trackpad pixel deltas are physical; the engine scrolls in logical (CSS) px.
                    MouseScrollDelta::PixelDelta(p) => (self.cursor_logical(p.x), self.cursor_logical(p.y)),
                };
                // Fire-and-forget one delta per wheel event; the engine accumulates the target,
                // clamps it to the page, and animates the scroll itself.
                if let Some(rt) = &self.state {
                    let tab = rt.tab.clone();
                    TOKIO_RT.spawn(async move {
                        let _ = tab
                            .send(TabCommand::MouseScroll {
                                delta_x: dx,
                                delta_y: dy,
                            })
                            .await;
                    });
                }
            }

            WindowEvent::ModifiersChanged(mods) => {
                self.modifiers = mods.state();
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
                if logical_key == Key::Character("l".into()) && self.modifiers.control_key() {
                    self.addr_focused = true;
                    self.url_input = self.current_url.clone();
                    self.update_title();
                    return;
                }

                // 't' (when not editing the address bar) dumps the full timing table to the terminal.
                if !self.addr_focused && logical_key == Key::Character("t".into()) {
                    gosub_shared::timing::dump(true);
                    return;
                }

                if self.addr_focused {
                    match &logical_key {
                        Key::Named(NamedKey::Enter) => self.navigate(),
                        Key::Named(NamedKey::Escape) => {
                            self.addr_focused = false;
                            self.url_input = self.current_url.clone();
                            self.update_title();
                        }
                        Key::Named(NamedKey::Backspace) => {
                            self.url_input.pop();
                            self.update_title();
                        }
                        _ => {
                            if let Some(t) = &text {
                                self.url_input.push_str(t.as_str());
                                self.update_title();
                            }
                        }
                    }
                }
            }

            _ => {}
        }
    }
}

// ── main ──────────────────────────────────────────────────────────────────────

fn main() {
    simple_logger::SimpleLogger::new()
        .with_level(log::LevelFilter::Warn)
        .env()
        .init()
        .unwrap_or_default();

    println!("[hint] press 't' in the window to print the timing table to this terminal");

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

    // The wgpu instance can be created here — it doesn't need a display handle.
    // Adapter + device creation is deferred to resumed() where a window (and
    // therefore a surface) is available to pass as a compatibility hint.
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());

    let compositor = Arc::new(DefaultCompositor::new({
        let p = proxy.clone();
        move || {
            let _ = p.send_event(());
        }
    }));

    let url_input = initial_url.clone();
    let current_url = initial_url.clone();

    let mut app = BrowserApp {
        instance,
        compositor,
        proxy,
        initial_url,
        state: None,
        url_input,
        addr_focused: false,
        current_url,
        modifiers: ModifiersState::empty(),
        cursor: PhysicalPosition::default(),
        viewport: (1024, 768),
        scale: 1.0,
    };

    event_loop.run_app(&mut app).expect("event loop");
}
