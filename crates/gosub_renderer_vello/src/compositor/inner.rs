use gosub_render_pipeline::common::browser_state::BrowserState;
use gosub_render_pipeline::common::TextureStore;
use gosub_render_pipeline::layering::layer::LayerId;
use vello::kurbo::Affine;
use vello::peniko::{Blob, ImageAlphaType, ImageData, ImageFormat};

pub fn vello_compositor(layer_ids: Vec<LayerId>, state: &BrowserState, texture_store: &TextureStore) -> vello::Scene {
    let mut scene = vello::Scene::new();

    for layer_id in layer_ids {
        compose_layer(&mut scene, layer_id, state, texture_store);
    }

    scene
}

pub fn compose_layer(scene: &mut vello::Scene, layer_id: LayerId, state: &BrowserState, texture_store: &TextureStore) {
    let Some(ref tile_list) = state.tile_list else {
        log::error!("No tile list found");
        return;
    };

    let tile_ids = tile_list.read().get_intersecting_tiles(layer_id, state.viewport);
    for tile_id in tile_ids {
        let tile_data = {
            let binding = tile_list.read();
            match binding.get_tile(tile_id) {
                None => {
                    log::warn!("Tile not found: {:?}", tile_id);
                    None
                }
                Some(tile) => match tile.texture_id {
                    None => {
                        log::error!("No texture found for tile: {:?}", tile_id);
                        None
                    }
                    Some(tid) => Some((tid, tile.rect)),
                },
            }
        };
        let Some((texture_id, tile_rect)) = tile_data else {
            continue;
        };

        let Some(texture) = texture_store.get(texture_id) else {
            log::error!("No texture found for tile: {:?}", tile_id);
            continue;
        };

        // This CPU-into-scene compositor only handles CPU tiles; GPU-resident tiles are blitted by
        // the backend's `composite_tiles` step instead.
        let Some(cpu) = texture.cpu_data() else {
            continue;
        };
        // peniko ImageFormat::Rgba8 expects [R, G, B, A]; convert from the tile's tagged
        // byte order (no-op when the rasterizer already produced RGBA).
        let rgba = texture.format.to_rgba(cpu).into_owned();
        let surface = ImageData {
            data: Blob::from(rgba),
            format: ImageFormat::Rgba8,
            alpha_type: ImageAlphaType::AlphaPremultiplied,
            width: texture.width as u32,
            height: texture.height as u32,
        };

        scene.draw_image(&surface, Affine::translate((tile_rect.x.round(), tile_rect.y.round())));
    }
}
