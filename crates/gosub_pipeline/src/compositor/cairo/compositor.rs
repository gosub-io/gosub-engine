use crate::common::browser_state::get_browser_state;
use crate::common::get_texture_store;
use crate::layering::layer::LayerId;
use gtk4::cairo;
use gtk4::cairo::ImageSurface;

pub fn cairo_compositor(cr: &cairo::Context, layer_ids: Vec<LayerId>) {
    for layer_id in layer_ids {
        compose_layer(cr, layer_id);
    }
}

pub fn compose_layer(cr: &cairo::Context, layer_id: LayerId) {
    let binding = get_browser_state();
    let state = binding.read();

    let Some(ref tile_list) = state.tile_list else {
        log::error!("No tile list found");
        return;
    };

    let tile_list_guard = tile_list.read();

    let tile_ids = tile_list_guard.get_intersecting_tiles(layer_id, state.viewport);

    for tile_id in tile_ids {
        let Some(tile) = tile_list_guard.get_tile(tile_id) else {
            log::warn!("Tile not found: {:?}", tile_id);
            continue;
        };

        let Some(texture_id) = tile.texture_id else {
            log::error!("No texture found for tile: {:?}", tile_id);
            continue;
        };

        let binding = get_texture_store();
        let texture_store = binding.read();

        let Some(texture) = texture_store.get(texture_id) else {
            log::error!("No texture found for tile: {:?}", tile_id);
            continue;
        };

        let surface = match ImageSurface::create_for_data(
            texture.data.clone(),
            cairo::Format::ARgb32,
            texture.width as i32,
            texture.height as i32,
            texture.width as i32 * 4,
        ) {
            Ok(s) => s,
            Err(e) => {
                log::error!("Failed to create image surface for tile {:?}: {:?}", tile_id, e);
                continue;
            }
        };

        cr.rectangle(tile.rect.x, tile.rect.y, tile.rect.width, tile.rect.height);
        if let Err(e) = cr.set_source_surface(surface, tile.rect.x, tile.rect.y) {
            log::warn!("Failed to set source surface for tile {:?}: {:?}", tile_id, e);
            continue;
        }
        if let Err(e) = cr.fill() {
            log::warn!("Failed to fill tile {:?}: {:?}", tile_id, e);
        }
    }
}
