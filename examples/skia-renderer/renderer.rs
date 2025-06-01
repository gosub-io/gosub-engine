use gosub_render_pipeline::common::browser_state::get_browser_state;
use gosub_render_pipeline::layering::layer::LayerId;
use gosub_render_pipeline::painter::Painter;
use gosub_render_pipeline::rasterizer::Rasterable;
use gosub_render_pipeline::rasterizer::skia::SkiaRasterizer;
use gosub_render_pipeline::tiler::TileState;

pub fn do_paint(layer_id: LayerId) {
    let binding = get_browser_state();
    let state = binding.read().unwrap();

    let Some(ref tile_list) = state.tile_list else {
        log::error!("No tile list found");
        return;
    };

    let painter = Painter::new(tile_list.read().unwrap().layer_list.clone());

    let tile_ids = tile_list
        .read()
        .unwrap()
        .get_intersecting_tiles(layer_id, state.viewport);
    for tile_id in tile_ids {
        // get tile
        let mut binding = tile_list.write().expect("Failed to get tile list");
        let Some(tile) = binding.get_tile_mut(tile_id) else {
            log::warn!("Tile not found: {:?}", tile_id);
            continue;
        };

        // if not dirty, no need to render and continue
        if tile.state == TileState::Clean || tile.state == TileState::Empty {
            continue;
        }

        // Paint all the elements in each tile
        for tiled_layout_element in &mut tile.elements {
            tiled_layout_element.paint_commands = painter.paint(tiled_layout_element);
        }
    }
}

pub fn do_rasterize(layer_id: LayerId) {
    let binding = get_browser_state();
    let state = binding.read().unwrap();

    let Some(ref tile_list) = state.tile_list else {
        log::error!("No tile list found");
        return;
    };

    let tile_ids = tile_list
        .read()
        .unwrap()
        .get_intersecting_tiles(layer_id, state.viewport);
    for tile_id in tile_ids {
        // get tile
        let mut binding = tile_list.write().expect("Failed to get tile list");
        let Some(tile) = binding.get_tile(tile_id) else {
            log::warn!("Tile not found: {:?}", tile_id);
            continue;
        };

        // if not dirty, no need to render and continue
        if tile.state == TileState::Clean || tile.state == TileState::Empty {
            continue;
        }

        let Some(tile) = binding.get_tile_mut(tile_id) else {
            log::warn!("Tile not found: {:?}", tile_id);
            continue;
        };

        // Rasterize the tile into a texture
        let rasterizer = SkiaRasterizer::new(/*state.dpi_scale_factor*/ 1.0);
        match rasterizer.rasterize(tile) {
            Some(texture_id) => {
                tile.texture_id = Some(texture_id);
                tile.state = TileState::Clean;
            }
            None => {
                log::warn!("Tile not rasterized. Seems empty {:?}", tile_id);
                tile.state = TileState::Empty;
            }
        }
    }
}
