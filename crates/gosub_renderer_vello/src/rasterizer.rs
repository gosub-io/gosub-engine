use gosub_fontmanager::ParleyFontSystem;
use gosub_interface::font_system::FontSystem;
use gosub_render_pipeline::common::geo::{Dimension, Rect as GeoRect};
use gosub_render_pipeline::common::media::MediaStore;
use gosub_render_pipeline::common::texture::TextureId;
use gosub_render_pipeline::common::TextureStore;
use gosub_render_pipeline::painter::commands::PaintCommand;
use gosub_render_pipeline::rasterizer::Rasterable;
use gosub_render_pipeline::render::backend::TileAnchor;
use gosub_render_pipeline::render::DEVICE_PIXEL_RATIO;
use gosub_render_pipeline::tiler::Tile;

use crate::backend::WgpuResources;
use parking_lot::Mutex;
use std::sync::Arc;
use vello::kurbo::{Affine, Rect, Vec2};
use vello::peniko::{Color, Fill, Mix};
use vello::{AaConfig, RenderParams, Scene};

/// The transform a promoted layer's commands draw under. Mirrors `anchored_tile_pos`: normal layers
/// scroll, fixed layers ignore scroll, sticky layers get the clamped catch-up offset.
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

/// How far outside the surface (CSS px) a command's box may lie and still be painted.
const CULL_MARGIN: f64 = 64.0;

/// Where a text command's ink can land. Glyphs are placed from the layout box's
/// origin by the shaped text's own positions, so they reach as far as the shaped
/// text is wide and tall, not as far as the box (a `line-height: 0` box has no
/// height at all), and a glyph overhangs its line by up to its font size. The box
/// is widened to the shaped text and padded by the largest font size in it.
fn text_ink_box(command: &gosub_render_pipeline::painter::commands::text::Text) -> GeoRect {
    let shaped = &command.shaped;
    let pad = shaped
        .runs
        .iter()
        .map(|run| run.font_size as f64)
        .fold(command.font_info.size, f64::max);
    GeoRect::new(
        command.rect.x - pad,
        command.rect.y - pad,
        command.rect.width.max(shaped.width as f64) + 2.0 * pad,
        command.rect.height.max(shaped.height as f64) + 2.0 * pad,
    )
}

/// Shared by the per-tile rasterizer (once per tile, translated to the tile) and the GPU-scene path
/// (once for the whole viewport, translated by `−scroll`). `size` bounds text layout; commands carry
/// pre-shaped glyph runs, so no font system is needed.
pub(crate) fn paint_commands_to_scene(
    scene: &mut Scene,
    commands: &[PaintCommand],
    size: Dimension,
    affine: Affine,
    scroll: (f64, f64),
    media_store: &MediaStore,
) {
    let (sx, sy) = scroll;
    // Only what lands on the surface is encoded. On the GPU-scene path this is called with the
    // whole page's commands for every scroll frame, and Vello's cost follows what the scene holds,
    // not what it shows: a long article (11.5k commands) took ~430ms a frame unculled. The margin
    // covers what paints outside its box - glyph overhang, borders - with room to spare.
    let visible = Rect::new(0.0, 0.0, size.width, size.height).inflate(CULL_MARGIN, CULL_MARGIN);
    let on_surface = |r: GeoRect, cur: Affine| {
        let bbox = cur.transform_rect_bbox(Rect::new(r.x, r.y, r.x + r.width, r.y + r.height));
        bbox.overlaps(visible)
    };
    // Starts at the caller's affine and is swapped to a layer's anchor transform between
    // PushLayer/PopLayer. The tile path never emits those, so it paints under the initial affine.
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
            PaintCommand::Svg(command) if !on_surface(command.rect.rect(), cur) => {}
            PaintCommand::Rectangle(command) if !on_surface(command.rect(), cur) => {}
            PaintCommand::Text(command) if !on_surface(text_ink_box(command), cur) => {}
            PaintCommand::Svg(command) => {
                svg::do_paint_svg(scene, command.media_id, &command.rect, cur, media_store);
            }
            PaintCommand::Rectangle(command) => {
                rectangle::do_paint_rectangle(scene, command, cur, media_store);
            }
            PaintCommand::Text(command) => {
                if let Err(e) = text::do_paint_text(scene, command, size, cur, media_store) {
                    log::warn!("Failed to paint text: {:?}", e);
                }
            }
        }
    }
}

pub struct VelloRasterizer {
    resources: Arc<WgpuResources>,
    /// Exposed to the layouter via `Rasterable::font_system()` so layout measures with the
    /// configured instance. Painting no longer needs it - commands carry pre-shaped glyph runs.
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
    /// Shared with the layouter so layout and render measure against the same instance.
    fn font_system(&self) -> Option<Arc<Mutex<dyn FontSystem>>> {
        Some(Arc::clone(&self.font_system))
    }

    fn rasterize(&self, tile: &Tile, texture_store: &mut TextureStore, media_store: &MediaStore) -> Option<TextureId> {
        // Tiles are rasterized at physical pixels, like the CPU rasterizers: the scene is built
        // in CSS pixels and drawn under the DPR scale into a texture `dpr` times the tile's size.
        // `composite_tiles` places them in physical pixels to match.
        let dpr = DEVICE_PIXEL_RATIO.load(std::sync::atomic::Ordering::Relaxed).max(1);
        let (width, height) = (tile.rect.width as u32 * dpr, tile.rect.height as u32 * dpr);

        let mut scene = Scene::new();

        let tile_size = Dimension::new(tile.rect.width, tile.rect.height);

        let clip = Rect::new(0.0, 0.0, tile_size.width, tile_size.height);
        scene.push_clip_layer(Fill::NonZero, Affine::IDENTITY, &clip);

        let affine = Affine::translate(Vec2::new(-tile.rect.x, -tile.rect.y));

        for element in &tile.elements {
            // The tile path applies opacity/anchor at composite, so per-element commands carry no
            // PushLayer/PopLayer - scroll is irrelevant here.
            paint_commands_to_scene(
                &mut scene,
                &element.paint_commands,
                tile_size,
                affine,
                (0.0, 0.0),
                media_store,
            );
        }

        scene.pop_layer();

        let scene = if dpr > 1 {
            let mut scaled = Scene::new();
            scaled.append(&scene, Some(Affine::scale(dpr as f64)));
            scaled
        } else {
            scene
        };

        let device: &vello::wgpu::Device = &self.resources.device;
        let queue: &vello::wgpu::Queue = &self.resources.queue;

        // The tile stays GPU-resident - no readback. The engine only ever sees the opaque id, which
        // it carries through the normal tile cache and hands back to `composite_tiles`.
        let texture = crate::gpu_tiles::create_tile_texture(device, width, height);

        let render_params = RenderParams {
            base_color: Color::new([0.0, 0.0, 0.0, 0.0]),
            width,
            height,
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
            width as usize,
            height as usize,
            gpu_id,
            gosub_render_pipeline::render::backend::PixelFormat::Rgba8,
        );

        Some(texture_id)
    }
}
