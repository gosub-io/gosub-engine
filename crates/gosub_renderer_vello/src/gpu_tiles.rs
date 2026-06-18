//! Shared GPU tile compositor for wgpu-based backends.
//!
//! Consolidation goal: one tile pipeline for every backend. The shared render pipeline lays out,
//! tiles, dirty-tracks and rasterizes the page; a CPU backend's tiles land in `Vec<u8>` and the
//! host blends them, while a GPU backend's tiles land in GPU textures and *this* compositor blits
//! the visible ones into the surface. Nothing here is Vello-specific — it only needs a wgpu device
//! and tile texture views — so it can move to a shared gpu-support crate and serve any wgpu backend.
//!
//! It does **no** tiling, rasterization, or caching: the engine owns all of that (the tile cache
//! keyed by content hash already holds GPU tile ids across frames). This step just composites the
//! tiles it is handed.
//!
//! Note: `copy_texture_to_texture` would be marginally simpler than this blit pass, but the surface
//! is host-created with only `RENDER_ATTACHMENT` (not `COPY_DST`), so a render pass is the portable
//! choice — and it gives correct premultiplied-alpha blending for free.

use vello::wgpu;

/// A placed, GPU-resident tile to composite: a texture view plus its page-space rectangle.
pub struct PlacedTileTex<'a> {
    pub view: &'a wgpu::TextureView,
    pub page_x: f32,
    pub page_y: f32,
    pub width: u32,
    pub height: u32,
}

/// Uniform handed to the blit shader for one tile: destination rect in target pixels plus the
/// target dimensions (so the vertex shader can map to clip space). 32 bytes, std140-friendly.
struct BlitUniform {
    dst: [f32; 4],      // x, y, w, h  (target pixels)
    viewport: [f32; 2], // target width, height
}

impl BlitUniform {
    fn to_bytes(&self) -> [u8; 32] {
        let mut out = [0u8; 32];
        let floats = [
            self.dst[0],
            self.dst[1],
            self.dst[2],
            self.dst[3],
            self.viewport[0],
            self.viewport[1],
            0.0,
            0.0,
        ];
        for (i, f) in floats.iter().enumerate() {
            out[i * 4..i * 4 + 4].copy_from_slice(&f.to_le_bytes());
        }
        out
    }
}

struct BlitPipeline {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    target_format: wgpu::TextureFormat,
}

const BLIT_WGSL: &str = r#"
struct Uniforms {
    dst: vec4<f32>,
    viewport: vec2<f32>,
    pad: vec2<f32>,
};
@group(0) @binding(0) var<uniform> u: Uniforms;
@group(0) @binding(1) var tex: texture_2d<f32>;
@group(0) @binding(2) var samp: sampler;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs(@builtin(vertex_index) vid: u32) -> VsOut {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0),
        vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0),
    );
    let c = corners[vid];
    let px = u.dst.xy + c * u.dst.zw;
    let ndc = vec2<f32>(px.x / u.viewport.x * 2.0 - 1.0, 1.0 - px.y / u.viewport.y * 2.0);
    var out: VsOut;
    out.pos = vec4<f32>(ndc, 0.0, 1.0);
    out.uv = c;
    return out;
}

@fragment
fn fs(in: VsOut) -> @location(0) vec4<f32> {
    return textureSample(tex, samp, in.uv);
}
"#;

impl BlitPipeline {
    fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("gpu-tiles-blit-shader"),
            source: wgpu::ShaderSource::Wgsl(BLIT_WGSL.into()),
        });

        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gpu-tiles-blit-bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("gpu-tiles-blit-pl"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });

        // Premultiplied-alpha "source over": tiles are rendered premultiplied with a transparent
        // background, so they composite correctly over the white-cleared surface.
        let blend = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
        };

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("gpu-tiles-blit-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(blend),
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

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("gpu-tiles-blit-sampler"),
            ..Default::default()
        });

        Self {
            pipeline,
            layout,
            sampler,
            target_format,
        }
    }
}

/// Stateless-ish GPU tile compositor: clears the target and blits the visible GPU tiles into it.
/// The only state is the lazily-built blit pipeline (keyed by target format).
#[derive(Default)]
pub struct GpuTileCompositor {
    blit: Option<BlitPipeline>,
}

impl GpuTileCompositor {
    /// Clear `target_view` to white and composite every tile in `tiles` at `page_pos - scroll`.
    #[allow(clippy::too_many_arguments)]
    pub fn composite(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target_view: &wgpu::TextureView,
        target_format: wgpu::TextureFormat,
        target_w: u32,
        target_h: u32,
        scroll_x: f32,
        scroll_y: f32,
        tiles: &[PlacedTileTex<'_>],
    ) {
        if self.blit.as_ref().map(|b| b.target_format) != Some(target_format) {
            self.blit = Some(BlitPipeline::new(device, target_format));
        }
        let Some(blit) = self.blit.as_ref() else {
            return;
        };

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("gpu-tiles-composite"),
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("gpu-tiles-composite-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&blit.pipeline);

            // Transient per-tile uniform buffers + bind groups, kept alive until the pass ends.
            let mut keep_alive: Vec<(wgpu::Buffer, wgpu::BindGroup)> = Vec::with_capacity(tiles.len());
            for tile in tiles {
                let uniform = BlitUniform {
                    dst: [
                        tile.page_x - scroll_x,
                        tile.page_y - scroll_y,
                        tile.width as f32,
                        tile.height as f32,
                    ],
                    viewport: [target_w as f32, target_h as f32],
                };
                let ubuf = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("gpu-tiles-uniform"),
                    size: 32,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                queue.write_buffer(&ubuf, 0, &uniform.to_bytes());

                let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("gpu-tiles-bg"),
                    layout: &blit.layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: ubuf.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::TextureView(tile.view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: wgpu::BindingResource::Sampler(&blit.sampler),
                        },
                    ],
                });
                let idx = keep_alive.len();
                keep_alive.push((ubuf, bind_group));
                pass.set_bind_group(0, &keep_alive[idx].1, &[]);
                pass.draw(0..6, 0..1);
            }
        }
        queue.submit(std::iter::once(encoder.finish()));
    }
}

/// Create an offscreen texture for a single rasterized tile, usable both as a Vello render target
/// and as a sampled source in the blit compositor.
pub(crate) fn create_tile_texture(device: &wgpu::Device, width: u32, height: u32) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("gpu-tile"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        // RENDER_ATTACHMENT + STORAGE_BINDING: Vello renders into it.
        // TEXTURE_BINDING: the blit pass samples it.
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use parking_lot::Mutex;
    use vello::kurbo::{Affine, Rect};
    use vello::peniko::{Color, Fill};
    use vello::{AaConfig, AaSupport, RenderParams, Renderer, RendererOptions, Scene};

    /// Render two solid-color tiles into GPU textures, composite them side by side with the shared
    /// compositor, read back, and assert each tile landed in the right place. Skips if no adapter.
    #[test]
    fn gpu_tile_compositor_smoke() {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let Ok(adapter) = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            compatible_surface: None,
            power_preference: wgpu::PowerPreference::default(),
            force_fallback_adapter: false,
        })) else {
            eprintln!("no wgpu adapter — skipping gpu_tile_compositor_smoke");
            return;
        };
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
            .expect("device");
        let renderer = Mutex::new(
            Renderer::new(
                &device,
                RendererOptions {
                    antialiasing_support: AaSupport::all(),
                    ..Default::default()
                },
            )
            .expect("renderer"),
        );

        // Rasterize one 256×256 tile of a given color into a GPU texture.
        let make_tile = |color: Color| -> wgpu::Texture {
            let mut scene = Scene::new();
            scene.fill(Fill::NonZero, Affine::IDENTITY, color, None, &Rect::new(0.0, 0.0, 256.0, 256.0));
            let tex = create_tile_texture(&device, 256, 256);
            let view = tex.create_view(&Default::default());
            renderer
                .lock()
                .render_to_texture(
                    &device,
                    &queue,
                    &scene,
                    &view,
                    &RenderParams {
                        base_color: Color::TRANSPARENT,
                        width: 256,
                        height: 256,
                        antialiasing_method: AaConfig::Area,
                    },
                )
                .expect("render tile");
            tex
        };

        let red = make_tile(Color::new([0.9, 0.1, 0.1, 1.0]));
        let blue = make_tile(Color::new([0.1, 0.1, 0.9, 1.0]));
        let red_view = red.create_view(&Default::default());
        let blue_view = blue.create_view(&Default::default());

        let (tw, th) = (512u32, 256u32);
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("smoke-target"),
            size: wgpu::Extent3d {
                width: tw,
                height: th,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let target_view = target.create_view(&Default::default());

        let tiles = [
            PlacedTileTex {
                view: &red_view,
                page_x: 0.0,
                page_y: 0.0,
                width: 256,
                height: 256,
            },
            PlacedTileTex {
                view: &blue_view,
                page_x: 256.0,
                page_y: 0.0,
                width: 256,
                height: 256,
            },
        ];

        let mut compositor = GpuTileCompositor::default();
        compositor.composite(
            &device,
            &queue,
            &target_view,
            wgpu::TextureFormat::Rgba8Unorm,
            tw,
            th,
            0.0,
            0.0,
            &tiles,
        );

        let pixels = read_back(&device, &queue, &target, tw, th);
        let at = |x: u32, y: u32| -> (u8, u8, u8) {
            let o = ((y * tw + x) * 4) as usize;
            (pixels[o], pixels[o + 1], pixels[o + 2])
        };
        let (r0, g0, b0) = at(128, 128);
        let (r1, g1, b1) = at(384, 128);
        eprintln!("left=({r0},{g0},{b0}) right=({r1},{g1},{b1})");
        let _ = image::save_buffer("/tmp/gpu_tile_compositor_smoke.png", &pixels, tw, th, image::ColorType::Rgba8);

        assert!(r0 > 180 && b0 < 80, "left tile should be red, got ({r0},{g0},{b0})");
        assert!(b1 > 180 && r1 < 80, "right tile should be blue, got ({r1},{g1},{b1})");
    }

    fn read_back(device: &wgpu::Device, queue: &wgpu::Queue, tex: &wgpu::Texture, w: u32, h: u32) -> Vec<u8> {
        let unpadded = w * 4;
        let padded = (unpadded + 255) & !255;
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("smoke-readback"),
            size: (padded * h) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            tex.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: Some(h),
                },
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        queue.submit(std::iter::once(encoder.finish()));
        let slice = buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
        rx.recv().unwrap().unwrap();
        let mapped = slice.get_mapped_range();
        let mut out = Vec::with_capacity((unpadded * h) as usize);
        for row in 0..h {
            let start = (row * padded) as usize;
            out.extend_from_slice(&mapped[start..start + unpadded as usize]);
        }
        drop(mapped);
        buffer.unmap();
        out
    }
}
