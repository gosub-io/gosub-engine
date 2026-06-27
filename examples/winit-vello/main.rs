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
use gosub_render_pipeline::render::backend::{anchored_tile_pos, blend_over_argb_u32, scale_premul_argb_u32, ExternalHandle};
use gosub_render_pipeline::render::{DefaultCompositor, Viewport};
use gosub_renderer_vello::{VelloBackend, WgpuContextProvider};
use once_cell::sync::Lazy;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
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
use winit::window::{Window, WindowAttributes, WindowId};

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

// ── wgpu context provider ─────────────────────────────────────────────────────

struct WinitWgpuContextProvider {
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    textures: RwLock<HashMap<u64, (wgpu::Texture, wgpu::TextureView)>>,
    next_id: AtomicU64,
}

impl WinitWgpuContextProvider {
    fn new(device: Arc<wgpu::Device>, queue: Arc<wgpu::Queue>) -> Self {
        Self {
            device,
            queue,
            textures: RwLock::new(HashMap::new()),
            next_id: AtomicU64::new(1),
        }
    }
}

impl WgpuContextProvider for WinitWgpuContextProvider {
    fn device(&self) -> &wgpu::Device {
        &self.device
    }

    fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }

    fn device_arc(&self) -> Arc<wgpu::Device> {
        Arc::clone(&self.device)
    }

    fn queue_arc(&self) -> Arc<wgpu::Queue> {
        Arc::clone(&self.queue)
    }

    fn create_texture(&self, width: u32, height: u32, format: wgpu::TextureFormat) -> u64 {
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("gosub-vello-texture"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.textures.write().insert(id, (texture, view));
        id
    }

    fn get_texture(&self, id: u64) -> Option<(wgpu::Texture, wgpu::TextureView)> {
        self.textures.read().get(&id).map(|(t, v)| (t.clone(), v.clone()))
    }

    fn remove_texture(&self, id: u64) {
        self.textures.write().remove(&id);
    }
}

// ── GPU window state ──────────────────────────────────────────────────────────

struct GpuState {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    pipeline: wgpu::RenderPipeline,
    bg_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
}

impl GpuState {
    fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(device, &self.config);
    }

    /// Upload a CPU RGBA buffer to a texture and blit it to the swap chain.
    ///
    /// Used only for the `ExternalHandle::TileCache` fallback (tile-rasterizing backends
    /// such as Cairo/Skia). Vello renders GPU-direct and takes the `WgpuTextureId` path,
    /// which blits its texture without this CPU round-trip.
    fn present_pixels(&self, device: &wgpu::Device, queue: &wgpu::Queue, rgba: &[u8], w: u32, h: u32) {
        if w == 0 || h == 0 {
            return;
        }
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("gosub-vello-tile-blit"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(w * 4),
                rows_per_image: Some(h),
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        self.present(device, queue, &view);
    }

    fn present(&self, device: &wgpu::Device, queue: &wgpu::Queue, view: &wgpu::TextureView) {
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f) | wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
            _ => return,
        };

        let frame_view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());

        let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.bg_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &frame_view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bg, &[]);
            pass.draw(0..3, 0..1);
        }

        queue.submit([encoder.finish()]);
        frame.present();
    }
}

fn build_blit_pipeline(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
) -> (wgpu::RenderPipeline, wgpu::BindGroupLayout) {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("blit"),
        source: wgpu::ShaderSource::Wgsl(BLIT_SHADER.into()),
    });

    let bg_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: None,
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    multisampled: false,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    });

    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: None,
        bind_group_layouts: &[Some(&bg_layout)],
        immediate_size: 0,
    });

    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("blit"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            buffers: &[],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    });

    (pipeline, bg_layout)
}

/// Full-screen triangle that blits a texture to the swap chain.
const BLIT_SHADER: &str = r#"
var<private> VERTS: array<vec2<f32>, 3> = array<vec2<f32>, 3>(
    vec2<f32>(-1.0, -1.0),
    vec2<f32>( 3.0, -1.0),
    vec2<f32>(-1.0,  3.0),
);

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    let p = VERTS[vi];
    var out: VsOut;
    out.pos = vec4<f32>(p, 0.0, 1.0);
    // Derive UV from the NDC position so the whole source texture is stretched across the
    // surface regardless of their relative sizes. This matters when the source is a logical-px
    // frame (on a fractionally scaled display) and the surface is larger physical px; mapping
    // texel-for-pixel would otherwise leave the texture 1:1 in a corner. When sizes match it is
    // identical to a 1:1 blit. Y is flipped (NDC +1 = top of screen = texture row 0).
    out.uv = vec2<f32>(p.x * 0.5 + 0.5, p.y * -0.5 + 0.5);
    return out;
}

@group(0) @binding(0) var t: texture_2d<f32>;
@group(0) @binding(1) var s: sampler;

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    return textureSample(t, s, in.uv);
}
"#;

// ── Lazy GPU + engine state (created in resumed()) ────────────────────────────

struct RuntimeState {
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    context: Arc<WinitWgpuContextProvider>,
    gpu: GpuState,
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
    compositor: Arc<RwLock<DefaultCompositor>>,
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
    scroll: (f32, f32),
    page_height: f32,
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
        let mut s = self.url_input.clone();
        if !s.starts_with("http://") && !s.starts_with("https://") {
            s = format!("https://{s}");
        }
        let Ok(_) = Url::parse(&s) else { return };
        self.url_input = s.clone();
        self.addr_focused = false;
        self.scroll = (0.0, 0.0);
        self.update_title();
        let tab = rt.tab.clone();
        TOKIO_RT.spawn(async move {
            let _ = tab.send(TabCommand::Navigate { url: s }).await;
            let _ = tab.send(TabCommand::ResumeDrawing { fps: 30 }).await;
        });
    }

    fn update_title(&self) {
        let Some(rt) = &self.state else { return };
        let title = if self.addr_focused {
            format!("URL: {} — Gosub (Enter to navigate, Esc to cancel)", self.url_input)
        } else {
            format!("Gosub Browser — {}", self.current_url)
        };
        rt.gpu.window.set_title(&title);
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
        let Some(handle) = self.compositor.read().frame_for(tab_id) else {
            return;
        };

        match handle {
            // GPU path — the engine renders the whole display list into a single GPU
            // texture (Vello's `raster_strategy() == None`) and hands us its id. Blit it
            // straight to the swap chain, no CPU round-trip.
            ExternalHandle::WgpuTextureId { id, .. } => {
                let rt = self.state.as_ref().unwrap();
                if let Some((_, view)) = rt.context.get_texture(id) {
                    rt.gpu.present(&rt.device, &rt.queue, &view);
                }
            }

            // CPU tile path — fallback for tile-rasterizing backends (Cairo/Skia). Composite
            // the visible tiles into an RGBA buffer, then upload + blit it via wgpu.
            ExternalHandle::TileCache {
                tiles,
                dpr,
                viewport_width,
                viewport_height,
                page_height,
                ..
            } => {
                self.page_height = page_height;

                let dpr_f = dpr as f32;
                let w = (viewport_width * dpr) as usize;
                let h = (viewport_height * dpr) as usize;
                if w == 0 || h == 0 {
                    return;
                }
                // Use the locally-tracked scroll so scrolling feels instant without waiting
                // for the engine to acknowledge the scroll command.
                // Opaque white background: a valid premultiplied base for source-over blending.
                // Each u32 holds [R, G, B, A] little-endian (R in the low byte).
                let mut buf = vec![0xFFFF_FFFFu32; w * h];
                for tile in tiles.iter() {
                    // Resolve the tile's viewport position in CSS px — this handles scroll, fixed
                    // and sticky uniformly — then scale to device px.
                    let (vx, vy) = anchored_tile_pos(
                        tile.page_x as f64,
                        tile.page_y as f64,
                        self.scroll.0 as f64,
                        self.scroll.1 as f64,
                        tile.anchor,
                    );
                    let screen_x = (vx * dpr_f as f64) as i64;
                    let screen_y = (vy * dpr_f as f64) as i64;
                    let tw = tile.width as i64;
                    let th = tile.height as i64;
                    if screen_x >= w as i64 || screen_y >= h as i64 || screen_x + tw <= 0 || screen_y + th <= 0 {
                        continue;
                    }
                    let tile_start_col = (-screen_x).max(0) as usize;
                    let tile_start_row = (-screen_y).max(0) as usize;
                    let dst_x = screen_x.max(0) as usize;
                    let dst_y0 = screen_y.max(0) as usize;
                    let tw = tw as usize;
                    let th = th as usize;
                    // Normalize to [R, G, B, A] regardless of which rasterizer produced the tile
                    // (Cargo feature unification may select Cairo's ARGB32 over Vello's RGBA).
                    let tile_data = tile.format.to_rgba(&tile.data);
                    let tile_u32 =
                        unsafe { std::slice::from_raw_parts(tile_data.as_ptr() as *const u32, tile_data.len() / 4) };
                    for tile_row in tile_start_row..th {
                        let dst_y = dst_y0 + (tile_row - tile_start_row);
                        if dst_y >= h {
                            break;
                        }
                        let copy_w = (tw - tile_start_col).min(w - dst_x);
                        if copy_w == 0 {
                            break;
                        }
                        let src_off = tile_row * tw + tile_start_col;
                        let dst_off = dst_y * w + dst_x;
                        for col in 0..copy_w {
                            buf[dst_off + col] = blend_over_argb_u32(scale_premul_argb_u32(tile_u32[src_off + col], tile.opacity), buf[dst_off + col]);
                        }
                    }
                }

                let mut rgba = Vec::with_capacity(w * h * 4);
                for &px in &buf {
                    rgba.extend_from_slice(&[
                        (px & 0xFF) as u8,
                        ((px >> 8) & 0xFF) as u8,
                        ((px >> 16) & 0xFF) as u8,
                        255,
                    ]);
                }

                let rt = self.state.as_ref().unwrap();
                rt.gpu.present_pixels(&rt.device, &rt.queue, &rgba, w as u32, h as u32);
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

        // ── 2. Create surface (needs the window's display handle) ─────────────
        let surface = match self.instance.create_surface(window.clone()) {
            Ok(s) => s,
            Err(e) => {
                log::error!("wgpu surface: {e}");
                return;
            }
        };

        // ── 3. Request adapter WITH surface compatibility hint ─────────────────
        // This is the key step: without a surface hint, wgpu may return an adapter
        // that can't render to the Wayland/X11 surface, causing `get_current_texture`
        // to silently fail every frame and the window to never become visible.
        let adapter = match TOKIO_RT.block_on(self.instance.request_adapter(&wgpu::RequestAdapterOptions {
            compatible_surface: Some(&surface),
            power_preference: wgpu::PowerPreference::default(),
            force_fallback_adapter: false,
        })) {
            Ok(a) => a,
            Err(e) => {
                log::error!("no wgpu adapter compatible with surface: {e}");
                return;
            }
        };

        let (device, queue) = match TOKIO_RT.block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())) {
            Ok(dq) => dq,
            Err(e) => {
                log::error!("wgpu device: {e}");
                return;
            }
        };
        let device = Arc::new(device);
        let queue = Arc::new(queue);

        // ── 4. Configure surface ──────────────────────────────────────────────
        let caps = surface.get_capabilities(&adapter);
        // Prefer a NON-sRGB swapchain format. Both the Vello GPU texture and the CPU tile blits
        // already contain sRGB-encoded bytes; presenting through an sRGB surface format would make
        // the hardware sRGB-encode them a second time, washing colors out (orange → yellow) and
        // brightening anti-aliased glyph edges so text looks too thin. A plain Unorm surface passes
        // the bytes straight through to the (sRGB) display.
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| !f.is_srgb())
            .or_else(|| caps.formats.first().copied())
            .unwrap_or(wgpu::TextureFormat::Bgra8Unorm);
        let alpha_mode = caps
            .alpha_modes
            .first()
            .copied()
            .unwrap_or(wgpu::CompositeAlphaMode::Auto);
        let size = window.inner_size();
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode,
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);

        let (pipeline, bg_layout) = build_blit_pipeline(&device, format);
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let gpu = GpuState {
            window,
            surface,
            config,
            pipeline,
            bg_layout,
            sampler,
        };

        // ── 5. Build VelloBackend and engine ──────────────────────────────────
        let context = Arc::new(WinitWgpuContextProvider::new(device.clone(), queue.clone()));
        let backend = match VelloBackend::new(context.clone()) {
            Ok(b) => b,
            Err(e) => {
                log::error!("VelloBackend: {e}");
                return;
            }
        };

        let mut engine = GosubEngine::<DefaultRenderConfig<_>>::new(None, Arc::new(backend), self.compositor.clone());
        let _join = engine.start().expect("engine start");

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
        self.scale = gpu.window.scale_factor();
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
            let _ = nav_tab.send(TabCommand::ResumeDrawing { fps: 30 }).await;
        });

        self.state = Some(RuntimeState {
            device,
            queue,
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
            rt.gpu.window.request_redraw();
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
            rt.gpu.window.request_redraw();
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
                    rt.gpu.resize(&rt.device, width, height);
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
                self.scroll = (0.0, 0.0);
            }

            // The window moved to a display with a different scale (e.g. dragged between
            // monitors). Re-derive logical size from the new scale and re-send the viewport.
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                self.scale = scale_factor;
                if let Some(rt) = &self.state {
                    let size = rt.gpu.window.inner_size();
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
                let max_y = (self.page_height - self.viewport.1 as f32).max(0.0);
                self.scroll.0 = (self.scroll.0 + dx).max(0.0);
                self.scroll.1 = (self.scroll.1 + dy).clamp(0.0, max_y);
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

    let compositor = Arc::new(RwLock::new(DefaultCompositor::new({
        let p = proxy.clone();
        move || {
            let _ = p.send_event(());
        }
    })));

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
        scroll: (0.0, 0.0),
        page_height: 0.0,
        viewport: (1024, 768),
        scale: 1.0,
    };

    event_loop.run_app(&mut app).expect("event loop");
}
