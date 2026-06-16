use gosub_fontmanager::ParleyFontSystem;
use gosub_interface::font_system::FontSystem;
use gosub_render_pipeline::common::geo::Dimension;
use gosub_render_pipeline::common::media::MediaStore;
use gosub_render_pipeline::common::texture::TextureId;
use gosub_render_pipeline::common::TextureStore;
use gosub_render_pipeline::painter::commands::PaintCommand;
use gosub_render_pipeline::rasterizer::Rasterable;
use gosub_render_pipeline::tiler::{Tile, TileId};

use crate::backend::WgpuResources;
use parking_lot::Mutex;
use std::sync::Arc;
use vello::kurbo::{Affine, Rect, Vec2};
use vello::peniko::{Color, Fill};
use vello::wgpu::{Texture, TextureDescriptor, TextureDimension, TextureFormat, TextureUsages};
use vello::{AaConfig, RenderParams, Scene};

mod brush;
mod rectangle;
mod svg;
mod text;

pub struct VelloRasterizer {
    resources: Arc<WgpuResources>,
    /// The engine's shared font system. Vello draws glyphs through Parley, so it downcasts this to
    /// `ParleyFontSystem` at draw time; a non-Parley font system means text simply isn't rendered
    /// (that backend↔font-system pairing doesn't make sense).
    font_system: Arc<Mutex<dyn FontSystem>>,
}

impl VelloRasterizer {
    /// Create a rasterizer with its own Parley font system.
    pub fn new(resources: Arc<WgpuResources>) -> Self {
        Self::with_font_system(resources, Arc::new(Mutex::new(ParleyFontSystem::new())))
    }

    /// Create a rasterizer that shares an existing font system.
    pub fn with_font_system(resources: Arc<WgpuResources>, font_system: Arc<Mutex<dyn FontSystem>>) -> Self {
        Self { resources, font_system }
    }
}

impl Rasterable for VelloRasterizer {
    /// Share the engine's font system with the layouter so layout and rendering measure/draw
    /// against the same instance.
    fn font_system(&self) -> Option<Arc<Mutex<dyn FontSystem>>> {
        Some(Arc::clone(&self.font_system))
    }

    fn rasterize(&self, tile: &Tile, texture_store: &mut TextureStore, media_store: &MediaStore) -> Option<TextureId> {
        let mut scene = Scene::new();

        let tile_size = Dimension::new(tile.rect.width, tile.rect.height);

        let clip = Rect::new(0.0, 0.0, tile_size.width, tile_size.height);
        scene.push_clip_layer(Fill::NonZero, Affine::IDENTITY, &clip);

        let affine = Affine::translate(Vec2::new(-tile.rect.x, -tile.rect.y));

        // Lock the font system once per tile and recover the concrete Parley system Vello draws
        // with. A non-Parley font system (e.g. Pango/Skia configured against the Vello backend)
        // means text can't be drawn here — log once and skip text commands.
        let mut font_guard = self.font_system.lock();
        let mut parley = font_guard.as_any_mut().downcast_mut::<ParleyFontSystem>();
        if parley.is_none() {
            log::warn!("Vello rasterizer: configured font system is not Parley; text will not render");
        }

        for element in &tile.elements {
            for command in &element.paint_commands {
                match command {
                    PaintCommand::Svg(command) => {
                        svg::do_paint_svg(&mut scene, command.media_id, &command.rect, affine, media_store);
                    }
                    PaintCommand::Rectangle(command) => {
                        rectangle::do_paint_rectangle(&mut scene, command, affine, media_store);
                    }
                    PaintCommand::Text(command) => {
                        if let Some(parley) = parley.as_deref_mut() {
                            let font_cx = parley.font_cx_mut();
                            if let Err(e) = text::do_paint_text(&mut scene, command, tile_size, affine, media_store, font_cx) {
                                log::warn!("Failed to paint text: {:?}", e);
                            }
                        }
                    }
                }
            }
        }
        drop(font_guard);

        scene.pop_layer();

        let device: &vello::wgpu::Device = &self.resources.device;
        let queue: &vello::wgpu::Queue = &self.resources.queue;

        let texture = create_offscreen_texture(device, tile_size.width as u32, tile_size.height as u32);

        let render_params = RenderParams {
            base_color: Color::new([0.0, 0.0, 0.0, 0.0]),
            width: tile.rect.width as u32,
            height: tile.rect.height as u32,
            antialiasing_method: AaConfig::Msaa16,
        };

        if let Err(e) = self.resources.renderer.lock().render_to_texture(
            device,
            queue,
            &scene,
            &texture.create_view(&Default::default()),
            &render_params,
        ) {
            log::error!("Vello render_to_texture failed: {:?}", e);
            return None;
        }

        let texture_data = read_texture_to_image(
            device,
            queue,
            &texture,
            tile_size.width as u32,
            tile_size.height as u32,
            tile.id,
        )?;

        let texture_id = texture_store.add(
            tile_size.width as usize,
            tile_size.height as usize,
            texture_data,
            gosub_render_pipeline::render::backend::PixelFormat::Rgba8,
        );

        Some(texture_id)
    }
}

fn create_offscreen_texture(device: &vello::wgpu::Device, width: u32, height: u32) -> Texture {
    device.create_texture(&TextureDescriptor {
        label: Some("Tile texture"),
        size: vello::wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format: TextureFormat::Rgba8Unorm,
        usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC | TextureUsages::STORAGE_BINDING,
        view_formats: &[],
    })
}

fn read_texture_to_image(
    device: &vello::wgpu::Device,
    queue: &vello::wgpu::Queue,
    texture: &Texture,
    width: u32,
    height: u32,
    _id: TileId,
) -> Option<Vec<u8>> {
    let unpadded_bytes_per_row = width * 4;
    let padded_bytes_per_row = (unpadded_bytes_per_row + 255) & !255;

    let buffer_size = (padded_bytes_per_row * height) as vello::wgpu::BufferAddress;
    let buffer = device.create_buffer(&vello::wgpu::BufferDescriptor {
        label: Some("Texture Read Buffer"),
        size: buffer_size,
        usage: vello::wgpu::BufferUsages::COPY_DST | vello::wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut encoder = device.create_command_encoder(&vello::wgpu::CommandEncoderDescriptor {
        label: Some("Texture Copy Encoder"),
    });
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        vello::wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: vello::wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded_bytes_per_row),
                rows_per_image: Some(height),
            },
        },
        vello::wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(std::iter::once(encoder.finish()));

    let buffer_slice = buffer.slice(..);
    let (sender, receiver) = std::sync::mpsc::channel();
    buffer_slice.map_async(vello::wgpu::MapMode::Read, move |result| {
        let _ = sender.send(result);
    });
    let _ = device.poll(vello::wgpu::PollType::wait_indefinitely());
    let Ok(Ok(())) = receiver.recv() else {
        log::error!("Failed to map texture buffer for reading");
        return None;
    };

    let padded = buffer_slice.get_mapped_range();
    let mut result = Vec::with_capacity((unpadded_bytes_per_row * height) as usize);
    for row in 0..height {
        let start = (row * padded_bytes_per_row) as usize;
        let end = start + unpadded_bytes_per_row as usize;
        result.extend_from_slice(&padded[start..end]);
    }
    drop(padded);
    buffer.unmap();

    Some(result)
}
