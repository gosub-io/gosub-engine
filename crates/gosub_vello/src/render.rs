use std::num::NonZeroUsize;
use std::sync::Arc;

use anyhow::anyhow;
use vello::{AaSupport, Renderer as VelloRenderer, RendererOptions as VelloRendererOptions};
use wgpu::{
    Adapter, Backends, CompositeAlphaMode, Device, Dx12Compiler, Gles3MinorVersion, Instance, InstanceDescriptor,
    PowerPreference, Queue, Surface, SurfaceConfiguration, TextureFormat,
};
use wgpu::util::{
    backend_bits_from_env, dx12_shader_compiler_from_env, gles_minor_version_from_env,
    power_preference_from_env,
};

use gosub_render_backend::WindowHandle;
use gosub_shared::types::Result;

pub mod window;

const DEFAULT_POWER_PREFERENCE: PowerPreference = PowerPreference::None;
const DEFAULT_BACKENDS: Backends = Backends::PRIMARY;
const DEFAULT_DX12COMPILER: Dx12Compiler = Dx12Compiler::Dxc {
    dxil_path: None,
    dxc_path: None,
};

const DEFAULT_GLES3_MINOR_VERSION: Gles3MinorVersion = Gles3MinorVersion::Automatic;

pub const RENDERER_CONF: VelloRendererOptions = VelloRendererOptions {
    surface_format: None,
    use_cpu: false,
    antialiasing_support: AaSupport {
        area: true,
        msaa8: true,
        msaa16: true,
    },
    num_init_threads: NonZeroUsize::new(1),
};

#[derive(Clone, Debug)]
pub struct Renderer {
    pub instance_adapter: Arc<InstanceAdapter>,
}

#[derive(Debug)]
pub struct InstanceAdapter {
    pub instance: Instance,
    pub adapter: Adapter,
    pub device: Device,
    pub queue: Queue,
}

pub struct RendererOptions {
    pub power_preference: Option<PowerPreference>,
    pub backends: Option<Backends>,
    pub dx12compiler: Option<Dx12Compiler>,
    pub gles3minor_version: Option<Gles3MinorVersion>,
    #[cfg(not(target_arch = "wasm32"))]
    pub adapter: Option<String>,
    #[cfg(not(target_arch = "wasm32"))]
    pub crash_on_invalid_adapter: bool,
}

impl Default for RendererOptions {
    fn default() -> Self {
        Self {
            power_preference: power_preference_from_env(),
            backends: backend_bits_from_env(),
            dx12compiler: dx12_shader_compiler_from_env(),
            gles3minor_version: gles_minor_version_from_env(),
            #[cfg(not(target_arch = "wasm32"))]
            adapter: std::env::var("WGPU_ADAPTER_NAME").ok(),
            #[cfg(not(target_arch = "wasm32"))]
            crash_on_invalid_adapter: false,
        }
    }
}

struct RenderConfig {
    pub power_preference: PowerPreference,
    pub backends: Backends,
    pub dx12compiler: Dx12Compiler,
    pub gles3minor_version: Gles3MinorVersion,
    #[cfg(not(target_arch = "wasm32"))]
    pub adapter: Option<String>,
    #[cfg(not(target_arch = "wasm32"))]
    pub crash_on_invalid_adapter: bool,
}

impl From<RendererOptions> for RenderConfig {
    fn from(opts: RendererOptions) -> Self {
        Self {
            power_preference: opts
                .power_preference
                .unwrap_or(power_preference_from_env().unwrap_or(DEFAULT_POWER_PREFERENCE)),
            backends: opts
                .backends
                .unwrap_or(backend_bits_from_env().unwrap_or(DEFAULT_BACKENDS)),
            dx12compiler: opts
                .dx12compiler
                .unwrap_or(dx12_shader_compiler_from_env().unwrap_or(DEFAULT_DX12COMPILER)),
            gles3minor_version: opts
                .gles3minor_version
                .unwrap_or(gles_minor_version_from_env().unwrap_or(DEFAULT_GLES3_MINOR_VERSION)),
            #[cfg(not(target_arch = "wasm32"))]
            adapter: opts.adapter,
            #[cfg(not(target_arch = "wasm32"))]
            crash_on_invalid_adapter: opts.crash_on_invalid_adapter,
        }
    }
}

impl Renderer {
    pub async fn new(opts: RendererOptions) -> Result<Self> {
        let config = RenderConfig::from(opts);

        Ok(Self {
            instance_adapter: Arc::new(Self::get_adapter(config).await?),
        })
    }

    async fn get_adapter(config: RenderConfig) -> Result<InstanceAdapter> {
        let instance = Instance::new(InstanceDescriptor {
            backends: config.backends,
            dx12_shader_compiler: config.dx12compiler,
            gles_minor_version: config.gles3minor_version,
            ..Default::default()
        });

        #[cfg(not(target_arch = "wasm32"))]
        let mut adapter = config.adapter.and_then(|adapter_name| {
            let adapters = instance.enumerate_adapters(Backends::all());
            let adapter_name = adapter_name.to_lowercase();

            let mut chosen_adapter = None;
            for adapter in adapters {
                let info = adapter.get_info();

                if info.name.to_lowercase().contains(&adapter_name) {
                    chosen_adapter = Some(adapter);
                    break;
                }
            }

            if chosen_adapter.is_none() && config.crash_on_invalid_adapter {
                eprintln!("No adapter found with name: {}", adapter_name);
                std::process::exit(1);
            }

            chosen_adapter
        });

        #[cfg(target_arch = "wasm32")]
        let mut adapter = None;

        if adapter.is_none() {
            adapter = instance
                .request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: config.power_preference,
                    force_fallback_adapter: false,
                    compatible_surface: None,
                })
                .await;
        }

        if adapter.is_none() {
            adapter = instance
                .request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: config.power_preference,
                    force_fallback_adapter: true,
                    compatible_surface: None,
                })
                .await;
        }

        let adapter = adapter.ok_or(anyhow!("No adapter found"))?;

        let info = adapter.get_info();

        let mut features = adapter.features();

        if info.device_type == wgpu::DeviceType::DiscreteGpu {
            features -= wgpu::Features::MAPPABLE_PRIMARY_BUFFERS;
        }

        features -= wgpu::Features::RAY_QUERY;
        features -= wgpu::Features::RAY_TRACING_ACCELERATION_STRUCTURE;

        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: None,
                    required_features: Default::default(),
                    required_limits: Default::default(),
                    memory_hints: Default::default(),
                },
                None,
            )
            .await
            .map_err(|e| anyhow!(e.to_string()))?;

        Ok(InstanceAdapter {
            instance,
            adapter,
            device,
            queue,
        })
    }
}

pub struct SurfaceWrapper<'a> {
    pub surface: Surface<'a>,
    pub config: SurfaceConfiguration,
}

impl InstanceAdapter {
    pub fn create_renderer(&self, surface_format: Option<TextureFormat>) -> Result<VelloRenderer> {
        let mut conf = RENDERER_CONF;
        conf.surface_format = surface_format;

        VelloRenderer::new(&self.device, conf).map_err(|e| anyhow!(e.to_string()))
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

        let surface = SurfaceWrapper { surface, config };

        self.configure_surface(&surface);

        Ok(surface)
    }

    pub fn resize_surface(&self, surface: &mut SurfaceWrapper, width: u32, height: u32) {
        surface.config.width = width;
        surface.config.height = height;
        self.configure_surface(surface);
    }

    fn configure_surface(&self, surface: &SurfaceWrapper) {
        surface.surface.configure(&self.device, &surface.config);
    }
}
