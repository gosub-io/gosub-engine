//! winit + wgpu window presentation glue for the Gosub Vello backend.
//!
//! An embedder that wants a winit window backed by the GPU (`VelloBackend`) needs two pieces of
//! boilerplate that are the same for every such app:
//!
//! - [`WinitWgpuContextProvider`] — the [`WgpuContextProvider`] the Vello backend renders through,
//!   owning the shared `wgpu` device/queue and a registry of engine-created textures.
//! - [`GpuPresenter`] — a wgpu surface + full-screen blit pipeline that puts a texture (Vello's
//!   GPU frame) or a CPU RGBA buffer (the tile fallback) onto the window's swap chain.
//!
//! [`GpuPresenter::new`] also performs the fiddly, easy-to-get-wrong adapter/surface setup: it
//! selects an adapter that is compatible with the window's surface (see its docs for the Wayland
//! trap) and a non-sRGB swap-chain format, so the embedder just creates a window and calls it.

use gosub_renderer_vello::WgpuContextProvider;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use vello::wgpu;
use winit::window::Window;

// ── WgpuContextProvider ───────────────────────────────────────────────────────

/// A [`WgpuContextProvider`] backed by a shared `wgpu` device/queue and an in-memory registry of
/// engine-created textures (keyed by an opaque id the engine hands back inside a `WgpuTextureId`
/// external handle).
pub struct WinitWgpuContextProvider {
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    textures: RwLock<HashMap<u64, (wgpu::Texture, wgpu::TextureView)>>,
    next_id: AtomicU64,
}

impl WinitWgpuContextProvider {
    pub fn new(device: Arc<wgpu::Device>, queue: Arc<wgpu::Queue>) -> Self {
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

// ── GpuPresenter ──────────────────────────────────────────────────────────────

/// A wgpu surface plus a full-screen blit pipeline that presents a texture (or a CPU RGBA buffer)
/// to a winit window.
///
/// Holds its own `Arc` device/queue so [`present`](Self::present) / [`present_rgba`](Self::present_rgba)
/// / [`resize`](Self::resize) take no extra plumbing; clone them out with [`device`](Self::device) /
/// [`queue`](Self::queue) to build a [`WinitWgpuContextProvider`] for the same GPU.
pub struct GpuPresenter {
    window: Arc<Window>,
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    pipeline: wgpu::RenderPipeline,
    bg_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
}

impl GpuPresenter {
    /// Create a surface for `window`, pick a surface-compatible adapter + device, configure a
    /// non-sRGB swap chain, and build the blit pipeline.
    ///
    /// Adapter selection passes `compatible_surface`: **without** that hint wgpu may return an
    /// adapter that cannot render to the Wayland/X11 surface, making `get_current_texture` silently
    /// fail every frame so the window never becomes visible.
    ///
    /// The swap-chain format is chosen **non-sRGB** where possible: both the Vello GPU texture and
    /// the CPU tile blits already hold sRGB-encoded bytes, so an sRGB surface format would encode
    /// them a second time — washing colors out (orange → yellow) and thinning anti-aliased glyph
    /// edges. A plain `Unorm` surface passes the bytes straight through to the (sRGB) display.
    ///
    /// This is `async` because `wgpu`'s adapter/device requests are; drive it on whatever runtime
    /// the embedder already has (e.g. `rt.block_on(GpuPresenter::new(..))`).
    pub async fn new(instance: &wgpu::Instance, window: Arc<Window>) -> anyhow::Result<Self> {
        let surface = instance.create_surface(window.clone())?;

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                compatible_surface: Some(&surface),
                power_preference: wgpu::PowerPreference::default(),
                force_fallback_adapter: false,
            })
            .await
            .map_err(|e| anyhow::anyhow!("no wgpu adapter compatible with surface: {e}"))?;

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor::default())
            .await
            .map_err(|e| anyhow::anyhow!("wgpu device: {e}"))?;
        let device = Arc::new(device);
        let queue = Arc::new(queue);

        let caps = surface.get_capabilities(&adapter);
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

        Ok(Self {
            window,
            device,
            queue,
            surface,
            config,
            pipeline,
            bg_layout,
            sampler,
        })
    }

    /// The window this presenter draws to.
    pub fn window(&self) -> &Arc<Window> {
        &self.window
    }

    /// The shared wgpu device (clone to build a [`WinitWgpuContextProvider`]).
    pub fn device(&self) -> &Arc<wgpu::Device> {
        &self.device
    }

    /// The shared wgpu queue (clone to build a [`WinitWgpuContextProvider`]).
    pub fn queue(&self) -> &Arc<wgpu::Queue> {
        &self.queue
    }

    /// Reconfigure the surface to a new physical size. No-op for a zero dimension.
    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
    }

    /// Blit `view` (e.g. Vello's GPU frame texture) to the swap chain.
    pub fn present(&self, view: &wgpu::TextureView) {
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f) | wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
            _ => return,
        };

        let frame_view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());

        let bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
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

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
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

        self.queue.submit([encoder.finish()]);
        frame.present();
    }

    /// Upload a CPU RGBA buffer to a texture and blit it to the swap chain.
    ///
    /// Used for the `ExternalHandle::TileCache` fallback (tile-rasterizing backends composited on
    /// the CPU). Vello renders GPU-direct and takes the [`present`](Self::present) path, avoiding
    /// this round-trip.
    pub fn present_rgba(&self, rgba: &[u8], w: u32, h: u32) {
        if w == 0 || h == 0 {
            return;
        }
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
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
        self.queue.write_texture(
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
        self.present(&view);
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
