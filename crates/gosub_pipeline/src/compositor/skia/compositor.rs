use crate::common::browser_state::get_browser_state;
use crate::common::get_texture_store;
use crate::layering::layer::LayerId;
use skia_safe::{AlphaType, ColorType, Data, ISize, ImageInfo};

pub fn skia_compositor(canvas: &skia_safe::Canvas, layer_ids: Vec<LayerId>) {
    for layer_id in layer_ids {
        compose_layer(canvas, layer_id);
    }
}

pub fn compose_layer(canvas: &skia_safe::Canvas, layer_id: LayerId) {
    let binding = get_browser_state();
    let Ok(state) = binding.read() else {
        log::error!("Failed to acquire browser state lock, skipping skia compose");
        return;
    };

    let Some(ref tile_list) = state.tile_list else {
        log::error!("No tile list found");
        return;
    };

    let Ok(tile_list_guard) = tile_list.read() else {
        log::error!("Failed to acquire tile list lock");
        return;
    };

    let tile_ids = tile_list_guard.get_intersecting_tiles(layer_id, state.viewport);
    let show_tilegrid = state.show_tilegrid;

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
        let Ok(texture_store) = binding.read() else {
            log::error!("Failed to acquire texture store lock for tile {:?}", tile_id);
            continue;
        };

        let Some(texture) = texture_store.get(texture_id) else {
            log::error!("No texture found for tile: {:?}", tile_id);
            continue;
        };

        let image_info = ImageInfo::new(
            ISize::new(texture.width as i32, texture.height as i32),
            ColorType::RGBA8888,
            AlphaType::Premul,
            None,
        );

        let data = Data::new_copy(texture.data.as_slice());

        let Some(img) = skia_safe::images::raster_from_data(&image_info, &data, texture.width * 4) else {
            log::error!("Failed to create Skia image for tile {:?}", tile_id);
            continue;
        };

        canvas.draw_image(&img, (tile.rect.x.round() as f32, tile.rect.y.round() as f32), None);

        if show_tilegrid {
            let mut paint = skia_safe::Paint::new(skia_safe::Color4f::new(1.0, 0.0, 0.0, 0.25), None);
            paint.set_stroke(true);

            let rect = skia_safe::Rect::from_xywh(
                tile.rect.x as f32,
                tile.rect.y as f32,
                tile.rect.width as f32,
                tile.rect.height as f32,
            );
            canvas.draw_rect(rect, &paint);
        }
    }
}
