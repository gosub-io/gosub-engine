use gosub_renderer_vello::WgpuContextProvider;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;
use vello::wgpu;
use wasm_bindgen::JsCast;

const BLIT_SHADER: &str = r#"
struct VertexOutput {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

// Full-screen triangle: 3 vertices cover the entire clip space without a vertex buffer.
@vertex
fn vs_main(@builtin(vertex_index) vid: u32) -> VertexOutput {
    let x = f32(i32(vid & 1u) * 2) - 1.0;
    let y = 1.0 - f32(i32(vid >> 1u) * 2);
    var out: VertexOutput;
    out.clip_pos = vec4<f32>(x, y, 0.0, 1.0);
    out.uv = vec2<f32>((x + 1.0) * 0.5, (1.0 - y) * 0.5);
    return out;
}

@group(0) @binding(0) var src_texture: texture_2d<f32>;
@group(0) @binding(1) var src_sampler: sampler;

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return textureSample(src_texture, src_sampler, in.uv);
}
"#;

/// wgpu context provider for WASM that renders to an HTML canvas via WebGPU.
///
/// Manages off-screen textures that VelloBackend renders into, then blits the
/// result to the canvas surface via a simple full-screen triangle pass.
pub struct WasmContextProvider {
    pub device: Arc<wgpu::Device>,
    pub queue: Arc<wgpu::Queue>,
    surface: wgpu::Surface<'static>,
    surface_format: wgpu::TextureFormat,
    blit_pipeline: wgpu::RenderPipeline,
    blit_bind_group_layout: wgpu::BindGroupLayout,
    blit_sampler: wgpu::Sampler,
    textures: RwLock<HashMap<u64, (wgpu::Texture, wgpu::TextureView)>>,
    next_id: RwLock<u64>,
}

// WASM is single-threaded; Send + Sync are safe here.
unsafe impl Send for WasmContextProvider {}
unsafe impl Sync for WasmContextProvider {}

impl WasmContextProvider {
    pub async fn new(canvas_id: &str, width: u32, height: u32) -> Result<Self, String> {
        let canvas = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.get_element_by_id(canvas_id))
            .and_then(|e| e.dyn_into::<web_sys::HtmlCanvasElement>().ok())
            .ok_or_else(|| format!("canvas '{}' not found", canvas_id))?;

        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::BROWSER_WEBGPU | wgpu::Backends::GL,
            ..Default::default()
        });

        let surface = instance
            .create_surface(wgpu::SurfaceTarget::Canvas(canvas))
            .map_err(|e| e.to_string())?;

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::default(),
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .ok_or("no suitable WebGPU adapter found")?;

        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("gosub-wasm"),
                    ..Default::default()
                },
                None,
            )
            .await
            .map_err(|e| e.to_string())?;

        let caps = surface.get_capabilities(&adapter);
        let surface_format = caps
            .formats
            .iter()
            .find(|f| f.is_srgb())
            .copied()
            .unwrap_or(caps.formats[0]);

        surface.configure(
            &device,
            &wgpu::SurfaceConfiguration {
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                format: surface_format,
                width,
                height,
                present_mode: wgpu::PresentMode::AutoVsync,
                alpha_mode: caps.alpha_modes[0],
                view_formats: vec![],
                desired_maximum_frame_latency: 2,
            },
        );

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("blit"),
            source: wgpu::ShaderSource::Wgsl(BLIT_SHADER.into()),
        });

        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("blit-bgl"),
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

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("blit-layout"),
            bind_group_layouts: &[&bgl],
            push_constant_ranges: &[],
        });

        let blit_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("blit-pipeline"),
            layout: Some(&pipeline_layout),
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
                    format: surface_format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        let blit_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("blit-sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        Ok(Self {
            device: Arc::new(device),
            queue: Arc::new(queue),
            surface,
            surface_format,
            blit_pipeline,
            blit_bind_group_layout: bgl,
            blit_sampler,
            textures: RwLock::new(HashMap::new()),
            next_id: RwLock::new(1),
        })
    }

    /// Blit the off-screen texture identified by `texture_id` to the canvas surface.
    pub fn present(&self, texture_id: u64) {
        let surface_tex = match self.surface.get_current_texture() {
            Ok(t) => t,
            Err(e) => {
                log::error!("wasm present: get_current_texture failed: {e}");
                return;
            }
        };
        let surface_view = surface_tex.texture.create_view(&wgpu::TextureViewDescriptor::default());

        let src_view = {
            let textures = self.textures.read();
            match textures.get(&texture_id) {
                Some((_, v)) => v.clone(),
                None => {
                    log::error!("wasm present: texture {texture_id} not found");
                    return;
                }
            }
        };

        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("blit-bg"),
            layout: &self.blit_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&src_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.blit_sampler),
                },
            ],
        });

        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("blit-encoder"),
        });

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("blit-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &surface_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                occlusion_query_set: None,
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.blit_pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.draw(0..3, 0..1);
        }

        self.queue.submit(std::iter::once(encoder.finish()));
        surface_tex.present();
    }
}

impl WgpuContextProvider for WasmContextProvider {
    fn device(&self) -> &wgpu::Device {
        &self.device
    }

    fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }

    fn create_texture(&self, width: u32, height: u32, format: wgpu::TextureFormat) -> u64 {
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("gosub-wasm-tex"),
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

        let mut id_lock = self.next_id.write();
        let id = *id_lock;
        *id_lock = id_lock.wrapping_add(1).max(1);
        drop(id_lock);

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
