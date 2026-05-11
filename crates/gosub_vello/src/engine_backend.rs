use parking_lot::Mutex;
use std::any::Any;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use anyhow::anyhow;
use vello::kurbo::Affine;
use vello::peniko::{Color as VelloColor, Fill};
use vello::wgpu;
use vello::{AaConfig, RenderParams, Renderer as VelloRenderer, Scene};

use gosub_engine::render::backend::{
    ErasedSurface, ExternalHandle, GpuPixelFormat, PresentMode, RenderBackend as EngineRenderBackend, RgbaImage,
    SurfaceSize,
};
use gosub_engine::render::DisplayItem;
use gosub_engine::BrowsingContext;

use crate::render::InstanceAdapter;

/// Off-screen Vello/wgpu surface used by the `gosub_engine::RenderBackend` path.
struct VelloEngineSurface {
    id: u64,
    size: SurfaceSize,
    frame_id: u64,
}

impl ErasedSurface for VelloEngineSurface {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
    fn size(&self) -> SurfaceSize {
        self.size
    }
}

/// Vello rendering backend for the new `gosub_engine::RenderBackend` path.
///
/// Owns its own wgpu device/queue and Vello renderer. Distinct from [`crate::VelloBackend`]
/// which implements the old `gosub_interface::RenderBackend` and obtains its wgpu context
/// from the windowing system via `WindowData`.
pub struct VelloEngineBackend {
    instance_adapter: Arc<InstanceAdapter>,
    renderer: Mutex<VelloRenderer>,
    textures: Mutex<HashMap<u64, (wgpu::Texture, wgpu::TextureView)>>,
    next_id: AtomicU64,
}

impl VelloEngineBackend {
    /// Creates a new backend, initialising the wgpu device/queue synchronously.
    pub fn new() -> anyhow::Result<Self> {
        let r = futures::executor::block_on(crate::render::Renderer::new())?;
        let ia = r.instance_adapter;
        let renderer = ia.create_renderer(None)?;

        Ok(Self {
            instance_adapter: ia,
            renderer: Mutex::new(renderer),
            textures: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
        })
    }
}

impl EngineRenderBackend for VelloEngineBackend {
    fn name(&self) -> &'static str {
        "vello"
    }

    fn create_surface(
        &self,
        size: SurfaceSize,
        _present: PresentMode,
    ) -> anyhow::Result<Box<dyn ErasedSurface + Send>> {
        let texture = self.instance_adapter.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("vello-engine-surface"),
            size: wgpu::Extent3d {
                width: size.width,
                height: size.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
            format: wgpu::TextureFormat::Rgba8Unorm,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.textures.lock().insert(id, (texture, view));

        Ok(Box::new(VelloEngineSurface { id, size, frame_id: 0 }))
    }

    fn render(&self, ctx: &mut BrowsingContext, surface: &mut dyn ErasedSurface) -> anyhow::Result<()> {
        let s = surface
            .as_any_mut()
            .downcast_mut::<VelloEngineSurface>()
            .ok_or_else(|| anyhow!("VelloEngineBackend: wrong surface type"))?;

        let vp = ctx.viewport();
        let offset_x = vp.x as f32;
        let offset_y = vp.y as f32;

        let mut scene = Scene::new();
        for item in ctx.render_list().items.iter() {
            match item {
                DisplayItem::Clear { color } => {
                    scene.fill(
                        Fill::NonZero,
                        Affine::IDENTITY,
                        VelloColor::new([color.r, color.g, color.b, color.a]),
                        None,
                        &vello::kurbo::Rect::new(0.0, 0.0, vp.width as f64, vp.height as f64),
                    );
                }
                DisplayItem::Rect { x, y, w, h, color } => {
                    let rx = (*x - offset_x) as f64;
                    let ry = (*y - offset_y) as f64;
                    scene.fill(
                        Fill::NonZero,
                        Affine::IDENTITY,
                        VelloColor::new([color.r, color.g, color.b, color.a]),
                        None,
                        &vello::kurbo::Rect::new(rx, ry, rx + *w as f64, ry + *h as f64),
                    );
                }
                DisplayItem::TextRun { .. } => {
                    // Text via the new engine API requires font data in DisplayItem.
                    // Will be addressed when the pipeline provides richer paint commands.
                    log::warn!("VelloEngineBackend: TextRun not yet implemented in new backend path");
                }
            }
        }

        let textures = self.textures.lock();
        let (_, view) = textures
            .get(&s.id)
            .ok_or_else(|| anyhow!("VelloEngineBackend: invalid surface id {}", s.id))?;

        self.renderer
            .lock()
            .render_to_texture(
                &self.instance_adapter.device,
                &self.instance_adapter.queue,
                &scene,
                view,
                &RenderParams {
                    base_color: VelloColor::WHITE,
                    width: s.size.width,
                    height: s.size.height,
                    antialiasing_method: AaConfig::Msaa16,
                },
            )
            .map_err(|e| anyhow!(e.to_string()))?;

        s.frame_id = s.frame_id.wrapping_add(1);
        Ok(())
    }

    fn snapshot(&self, _surface: &mut dyn ErasedSurface, _max_dim: u32) -> anyhow::Result<RgbaImage> {
        Err(anyhow!("VelloEngineBackend: GPU readback snapshot not yet implemented"))
    }

    fn external_handle(&self, surface: &mut dyn ErasedSurface) -> anyhow::Result<ExternalHandle> {
        let s = surface
            .as_any_mut()
            .downcast_mut::<VelloEngineSurface>()
            .ok_or_else(|| anyhow!("VelloEngineBackend: wrong surface type in external_handle"))?;

        Ok(ExternalHandle::WgpuTextureId {
            id: s.id,
            width: s.size.width,
            height: s.size.height,
            format: GpuPixelFormat::Rgba8UnormSrgb,
            frame_id: s.frame_id,
        })
    }
}

impl Drop for VelloEngineBackend {
    fn drop(&mut self) {
        self.textures.lock().clear();
    }
}
