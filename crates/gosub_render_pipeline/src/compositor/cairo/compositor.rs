use gtk4::cairo;
use gtk4::cairo::ImageSurface;
use crate::common::browser_state::get_browser_state;
use crate::layering::layer::LayerId;
use crate::common::get_texture_store;

pub fn cairo_compositor(cr: &cairo::Context, layer_ids: Vec<LayerId>) {
    for layer_id in layer_ids {
        compose_layer(cr, layer_id);
    }
}

pub fn compose_layer(cr: &cairo::Context, layer_id: LayerId) {
    let binding = get_browser_state();
    let state = binding.read().expect("Failed to get browser state");

    let tile_ids = state.tile_list.read().expect("Failed to get tile list").get_intersecting_tiles(layer_id, state.viewport);
    for tile_id in tile_ids {
        let binding = state.tile_list.write().expect("Failed to get tile list");
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

        let surface = ImageSurface::create_for_data(
            texture.data.clone(),       // Expensive, but we need to copy the data onto a new surface
            cairo::Format::ARgb32,
            texture.width as i32,
            texture.height as i32,
            texture.width as i32 * 4,
        ).expect("Failed to create image surface");

        cr.rectangle(
            tile.rect.x,
            tile.rect.y,
            tile.rect.height,
            tile.rect.width,
        );
        _ = cr.set_source_surface(surface, tile.rect.x, tile.rect.y);
        _ = cr.fill();
    }

}