use gosub_fontmanager::ParleyFontSystem;
use gosub_interface::font_system::FontSystem;
use gosub_render_pipeline::common::geo::Dimension;
use gosub_render_pipeline::common::media::MediaStore;
use gosub_render_pipeline::common::texture::TextureId;
use gosub_render_pipeline::common::TextureStore;
use gosub_render_pipeline::painter::commands::PaintCommand;
use gosub_render_pipeline::rasterizer::Rasterable;
use gosub_render_pipeline::render::backend::TileAnchor;
use gosub_render_pipeline::tiler::Tile;

use crate::backend::WgpuResources;
use parking_lot::Mutex;
use std::sync::Arc;
use vello::kurbo::{Affine, Rect, Vec2};
use vello::peniko::{Color, Fill, Mix};
use vello::{AaConfig, RenderParams, Scene};

/// The transform a promoted layer's commands draw under, given its anchor and the current scroll.
/// Mirrors `anchored_tile_pos`: normal layers scroll, fixed layers ignore scroll, sticky layers get
/// the clamped catch-up offset.
fn layer_affine(anchor: TileAnchor, sx: f64, sy: f64) -> Affine {
    match anchor {
        TileAnchor::Scroll => Affine::translate(Vec2::new(-sx, -sy)),
        TileAnchor::Fixed => Affine::IDENTITY,
        TileAnchor::Sticky(c) => {
            let (dx, dy) = c.offset(sx, sy);
            Affine::translate(Vec2::new(-sx + dx, -sy + dy))
        }
    }
}

mod brush;
mod rectangle;
mod svg;
mod text;

/// Translate a flat list of paint commands into a Vello scene.
///
/// Shared by the per-tile rasterizer (called once per tile, clipped + translated to the tile)
/// and the GPU-scene backend path (called once for the whole viewport, translated by `−scroll`).
/// `size` bounds text layout; `parley` is the concrete font system Vello draws glyphs through
/// (`None` skips text — a non-Parley font system can't render here).
pub(crate) fn paint_commands_to_scene(
    scene: &mut Scene,
    commands: &[PaintCommand],
    size: Dimension,
    affine: Affine,
    scroll: (f64, f64),
    media_store: &MediaStore,
    mut parley: Option<&mut ParleyFontSystem>,
) {
    let (sx, sy) = scroll;
    // The transform the current commands draw under. Starts at the caller's affine (per-tile
    // translate for the tile path, `−scroll` for the whole-page scene path) and is swapped to a
    // layer's anchor transform between PushLayer/PopLayer. The tile path never emits those, so it
    // simply paints everything under the initial affine.
    let mut cur = affine;
    // (transform to restore, whether we pushed an opacity group) for each open PushLayer.
    let mut stack: Vec<(Affine, bool)> = Vec::new();
    for command in commands {
        match command {
            PaintCommand::PushLayer { opacity, anchor } => {
                // Fade only when actually translucent (avoids a wasted offscreen group at α=1).
                let faded = *opacity < 1.0;
                if faded {
                    // Clip to the viewport so the group's backing buffer stays viewport-sized; the
                    // commands position themselves via `cur`, so the layer transform is identity.
                    let clip = Rect::new(0.0, 0.0, size.width, size.height);
                    scene.push_layer(Fill::NonZero, Mix::Normal, *opacity, Affine::IDENTITY, &clip);
                }
                stack.push((cur, faded));
                cur = layer_affine(*anchor, sx, sy);
            }
            PaintCommand::PopLayer => {
                if let Some((prev, faded)) = stack.pop() {
                    if faded {
                        scene.pop_layer();
                    }
                    cur = prev;
                }
            }
            PaintCommand::Svg(command) => {
                svg::do_paint_svg(scene, command.media_id, &command.rect, cur, media_store);
            }
            PaintCommand::Rectangle(command) => {
                rectangle::do_paint_rectangle(scene, command, cur, media_store);
            }
            PaintCommand::Text(command) => {
                if let Some(parley) = parley.as_deref_mut() {
                    if let Err(e) = text::do_paint_text(scene, command, size, cur, media_store, parley) {
                        log::warn!("Failed to paint text: {:?}", e);
                    }
                }
            }
        }
    }
}

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
            // The tile path applies opacity/anchor at composite, so per-element commands carry no
            // PushLayer/PopLayer — scroll is irrelevant here.
            paint_commands_to_scene(
                &mut scene,
                &element.paint_commands,
                tile_size,
                affine,
                (0.0, 0.0),
                media_store,
                parley.as_deref_mut(),
            );
        }
        drop(font_guard);

        scene.pop_layer();

        let device: &vello::wgpu::Device = &self.resources.device;
        let queue: &vello::wgpu::Queue = &self.resources.queue;

        // Render the tile straight into a GPU texture and keep it resident — no CPU readback. The
        // texture is registered in the backend's shared tile store; the engine carries the opaque
        // id through the normal tile cache and hands it back to `composite_tiles` for blitting.
        let texture = crate::gpu_tiles::create_tile_texture(device, tile_size.width as u32, tile_size.height as u32);

        let render_params = RenderParams {
            base_color: Color::new([0.0, 0.0, 0.0, 0.0]),
            width: tile.rect.width as u32,
            height: tile.rect.height as u32,
            antialiasing_method: AaConfig::Area,
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

        let gpu_id = self.resources.store_tile(texture);

        let texture_id = texture_store.add_gpu(
            tile_size.width as usize,
            tile_size.height as usize,
            gpu_id,
            gosub_render_pipeline::render::backend::PixelFormat::Rgba8,
        );

        Some(texture_id)
    }
}
