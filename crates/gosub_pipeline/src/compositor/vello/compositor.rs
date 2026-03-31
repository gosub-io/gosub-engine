use vello::kurbo::Affine;
use crate::common::browser_state::get_browser_state;
use crate::common::get_texture_store;
use crate::layering::layer::LayerId;
use vello::peniko::{Blob, Image, ImageFormat};

pub fn vello_compositor(layer_ids: Vec<LayerId>) -> vello::Scene {
    let mut scene = vello::Scene::new();

    for layer_id in layer_ids {
        compose_layer(&mut scene, layer_id);
    }

    scene
}

pub fn compose_layer(scene: &mut vello::Scene, layer_id: LayerId) {
    let binding = get_browser_state();
    let state = binding.read().expect("Failed to get browser state");

    let Some(ref tile_list) = state.tile_list else {
        log::error!("No tile list found");
        return;
    };

    let tile_ids = tile_list.read().expect("Failed to get tile list").get_intersecting_tiles(layer_id, state.viewport);
    for tile_id in tile_ids {
        let binding = tile_list.write().expect("Failed to get tile list");
        let Some(tile) = binding.get_tile(tile_id) else {
            log::warn!("Tile not found: {:?}", tile_id);
            continue;
        };

        let Some(texture_id) = tile.texture_id else {
            log::error!("No texture found for tile: {:?}", tile_id);
            continue;
        };

        let binding = get_texture_store();
        let texture_store = binding.read().expect("Failed to get texture store");

        let Some(texture) = texture_store.get(texture_id) else {
            log::error!("No texture found for tile: {:?}", tile_id);
            continue;
        };
        drop(texture_store);

        let surface = Image::new(
            Blob::from(texture.data.clone()),   // Don't clone :(
            ImageFormat::Rgba8,
            texture.width as u32,
            texture.height as u32,
        );

        scene.draw_image(
            &surface,
            Affine::translate((tile.rect.x.round(), tile.rect.y.round())),
        );
    }

}