use std::sync::Arc;

use anyhow::anyhow;
use vello::wgpu::{
    self, CompositeAlphaMode, Device, Queue, Surface, SurfaceConfiguration, Texture, TextureFormat, TextureView,
};
use vello::{AaSupport, Renderer as VelloRenderer, RendererOptions as VelloRendererOptions};

use gosub_interface::render_backend::WindowHandle;
use gosub_interface::types::Result;

pub mod window;

#[derive(Clone, Debug)]
pub struct Renderer {
    pub instance_adapter: Arc<InstanceAdapter>,
}

#[derive(Debug)]
pub struct InstanceAdapter {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: Device,
    pub queue: Queue,
}

impl Renderer {
    pub async fn new() -> Result<Self> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::from_env_or_default());

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::from_env().unwrap_or_default(),
                force_fallback_adapter: false,
                compatible_surface: None,
            })
            .await
            .map_err(|e| anyhow!(e.to_string()))?;

        let required_features = adapter.features() & (wgpu::Features::CLEAR_TEXTURE | wgpu::Features::PIPELINE_CACHE);

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: None,
                required_features,
                required_limits: wgpu::Limits::default(),
                experimental_features: wgpu::ExperimentalFeatures::disabled(),
                memory_hints: wgpu::MemoryHints::default(),
                trace: wgpu::Trace::Off,
            })
            .await
            .map_err(|e| anyhow!(e.to_string()))?;

        Ok(Self {
            instance_adapter: Arc::new(InstanceAdapter {
                instance,
                adapter,
                device,
                queue,
            }),
        })
    }
}

pub struct SurfaceWrapper<'a> {
    pub surface: Surface<'a>,
    pub config: SurfaceConfiguration,
    pub target_texture: Texture,
    pub target_view: TextureView,
}

impl InstanceAdapter {
    pub fn create_renderer(&self, _surface_format: Option<TextureFormat>) -> Result<VelloRenderer> {
        VelloRenderer::new(
            &self.device,
            VelloRendererOptions {
                use_cpu: false,
                antialiasing_support: AaSupport {
                    area: true,
                    msaa8: true,
                    msaa16: true,
                },
                num_init_threads: None,
                pipeline_cache: None,
            },
        )
        .map_err(|e| anyhow!(e.to_string()))
    }

    pub fn create_surface<'a>(
        &self,
        window: impl WindowHandle + 'a,
        width: u32,
        height: u32,
        present_mode: wgpu::PresentMode,
    ) -> Result<SurfaceWrapper<'a>> {
        let surface = self.instance.create_surface(window)?;
        let capabilities = surface.get_capabilities(&self.adapter);
        let format = capabilities
            .formats
            .into_iter()
            .find(|it| matches!(it, TextureFormat::Rgba8Unorm | TextureFormat::Bgra8Unorm))
            .ok_or(anyhow!("surface should support Rgba8Unorm or Bgra8Unorm"))?;

        let config = SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width,
            height,
            present_mode,
            desired_maximum_frame_latency: 2,
            alpha_mode: CompositeAlphaMode::Auto,
            view_formats: vec![],
        };

        let (target_texture, target_view) = create_targets(width, height, &self.device);

        let surface = SurfaceWrapper {
            surface,
            config,
            target_texture,
            target_view,
        };
        self.configure_surface(&surface);
        Ok(surface)
    }

    pub fn resize_surface(&self, surface: &mut SurfaceWrapper, width: u32, height: u32) {
        let (target_texture, target_view) = create_targets(width, height, &self.device);
        surface.config.width = width;
        surface.config.height = height;
        surface.target_texture = target_texture;
        surface.target_view = target_view;
        self.configure_surface(surface);
    }

    fn configure_surface(&self, surface: &SurfaceWrapper) {
        surface.surface.configure(&self.device, &surface.config);
    }
}

fn create_targets(width: u32, height: u32, device: &Device) -> (Texture, TextureView) {
    let target_texture = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
        format: TextureFormat::Rgba8Unorm,
        view_formats: &[],
    });
    let target_view = target_texture.create_view(&wgpu::TextureViewDescriptor::default());
    (target_texture, target_view)
}
