use skia_safe::{AlphaType, ColorType, Data, ISize, ImageInfo};
use crate::common::get_texture_store;
use crate::layering::layer::LayerId;
use crate::with_render_state;

pub fn skia_compositor(canvas: &skia_safe::Canvas, layer_ids: Vec<LayerId>) {
    for layer_id in layer_ids {
        compose_layer(canvas, layer_id);
    }
}

pub fn compose_layer(canvas: &skia_safe::canvas::Canvas, layer_id: LayerId) {
    let tile_list = with_render_state!(config, state => {
        let Some(ref tile_list) = state.tile_list else {
            log::error!("No tile list found");
            return;
        };

        tile_list
    });

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

        let image_info = ImageInfo::new(
            ISize::new(texture.width as i32, texture.height as i32),
            ColorType::RGBA8888,
            AlphaType::Premul,
            None,
        );

        #[allow(unsafe_code)]
        let data = unsafe { Data::new_bytes(&texture.data.as_slice()) };

        let img = skia_safe::images::raster_from_data(
            &image_info, &data, texture.width * 4
        ).unwrap();

        canvas.draw_image(
            &img,
            (tile.rect.x.round() as f32, tile.rect.y.round() as f32),
            None,
        );

        // Display rectangles around the tiles
        if state.show_tilegrid {
            let mut paint = skia_safe::Paint::new(
                skia_safe::Color4f::new(1.0, 0.0, 0.0, 0.25),
                None,
            );
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