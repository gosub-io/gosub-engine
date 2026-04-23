use gosub_engine_api::render::backends::vello::WgpuContextProvider;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

/// Implementation of the WgpuContextProvider trait using eframe's wgpu context.
/// It connects the eframe wgpu context to the Vello rendering backend so both can
/// share the same wgpu device and queue.
pub struct EguiWgpuContextProvider {
    /// The wgpu device.
    device: Arc<wgpu::Device>,
    /// The wgpu queue.
    queue: Arc<wgpu::Queue>,
    // The preferred texture format.
    // format: wgpu::TextureFormat,
    /// Map of texture IDs to their corresponding wgpu textures and views.
    textures: RwLock<HashMap<u64, (wgpu::Texture, wgpu::TextureView)>>,
    /// The next texture ID to use.
    next_texture_id: RwLock<u64>,
}

impl EguiWgpuContextProvider {
    /// Creates a new EguiWgpuContextProvider from the given eframe CreationContext.
    pub fn from_eframe(cc: &eframe::CreationContext<'_>) -> Option<Self> {
        let wgpu_render_state = cc
            .wgpu_render_state
            .as_ref()
            .expect("eframe wgpu_render_state is required");

        Some(Self {
            device: Arc::new(wgpu_render_state.device.clone()),
            queue: Arc::new(wgpu_render_state.queue.clone()),
            // format: wgpu_render_state.target_format,
            textures: RwLock::new(HashMap::new()),
            next_texture_id: RwLock::new(1),
        })
    }
}

impl WgpuContextProvider for EguiWgpuContextProvider {
    fn device(&self) -> &wgpu::Device {
        &self.device
    }

    fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }

    fn create_texture(&self, width: u32, height: u32, format: wgpu::TextureFormat) -> u64 {
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Gosub Vello Texture"),
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

        let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        let mut id_lock = self.next_texture_id.write().unwrap();
        let texture_store_id = *id_lock;
        *id_lock = id_lock.wrapping_add(1).max(1);
        drop(id_lock);

        let mut textures_lock = self.textures.write().unwrap();
        textures_lock.insert(texture_store_id, (texture, texture_view));
        drop(textures_lock);

        texture_store_id
    }

    fn get_texture(&self, id: u64) -> Option<(wgpu::Texture, wgpu::TextureView)> {
        let textures_lock = self.textures.read().unwrap();
        textures_lock.get(&id).map(|(tex, view)| (tex.clone(), view.clone()))
    }

    fn remove_texture(&self, id: u64) {
        let mut textures_lock = self.textures.write().unwrap();
        textures_lock.remove(&id);
    }
}
