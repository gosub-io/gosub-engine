use gosub_render_pipeline::common;
use gosub_render_pipeline::common::browser_state::{BrowserState, WireframeState};
use gosub_render_pipeline::common::geo::{Dimension, Rect};
use gosub_render_pipeline::common::media::MediaStore;
use gosub_render_pipeline::common::TextureStore;
use gosub_render_pipeline::compositor::Composable;
use gosub_render_pipeline::layering::layer::{LayerId, LayerList};
use gosub_render_pipeline::layouter::taffy::TaffyLayouter;
use gosub_render_pipeline::layouter::CanLayout;
use gosub_render_pipeline::painter::Painter;
use gosub_render_pipeline::rasterizer::Rasterable;
use gosub_render_pipeline::rendertree_builder::RenderTree;
use gosub_render_pipeline::tiler::{TileList, TileState};
use gosub_renderer_cairo::compositor::{CairoCompositor, CairoCompositorConfig};
use gosub_renderer_cairo::CairoRasterizer;
use gtk4::prelude::{
    AdjustmentExt, ApplicationExt, ApplicationExtManual, DrawingAreaExt, DrawingAreaExtManual, GtkWindowExt, WidgetExt,
};
use gtk4::{glib, Adjustment, Application, ApplicationWindow, DrawingArea, EventControllerMotion, ScrolledWindow};
use parking_lot::RwLock;
use std::sync::Arc;

const TILE_DIMENSION: f64 = 256.0;

const WINDOW_WIDTH: f64 = 1024.0;
const WINDOW_HEIGHT: f64 = 768.0;

fn main() {
    let doc = common::document::parser::document_from_json("file://.", "cm.json");
    let mut output = String::new();
    if let Err(e) = doc.print_tree(&mut output) {
        log::warn!("Failed to print tree: {:?}", e);
    }
    println!("{}", output);

    let doc = Arc::new(doc);

    let mut render_tree = RenderTree::new(doc.clone());
    render_tree.parse();

    let mut layouter = TaffyLayouter::new();
    let layout_tree = layouter.layout(render_tree, None, 1.0);
    layouter.print_tree();
    println!(
        "Layout width: {}, height: {}",
        layout_tree.root_dimension.width, layout_tree.root_dimension.height
    );

    let layer_list = LayerList::new(layout_tree);

    let mut tile_list = TileList::new(layer_list, Dimension::new(TILE_DIMENSION, TILE_DIMENSION));
    tile_list.generate();

    let app = Application::builder().application_id("io.gosub.renderer").build();

    let layer_count = tile_list.layer_list.layer_ids.read().len();
    let browser_state = BrowserState {
        visible_layer_list: vec![true; layer_count],
        wireframed: WireframeState::None,
        debug_hover: false,
        current_hovered_element: None,
        tile_list: Some(RwLock::new(tile_list)),
        show_tilegrid: true,
        viewport: Rect::ZERO,
        dpi_scale_factor: 1.0,
    };

    let browser_state = Arc::new(RwLock::new(browser_state));
    let texture_store = Arc::new(RwLock::new(TextureStore::new()));
    let media_store = Arc::new(RwLock::new(MediaStore::new()));

    let bs_clone = browser_state.clone();
    let ts_clone = texture_store.clone();
    let ms_clone = media_store.clone();

    app.connect_activate(move |app| {
        build_ui(app, bs_clone.clone(), ts_clone.clone(), ms_clone.clone());
    });

    println!(
        r"
---------------------------------------
    Gosub Rendering Pipeline Concept
---------------------------------------

Available key commands:
  0-9   Toggle layer [0-9]
  w     Toggle wireframe
  d     Toggle debug hover
  t     Toggle tile grid
  s     Print hover timing stats to stdout

"
    );

    app.run();
}

fn build_ui(
    app: &Application,
    browser_state: Arc<RwLock<BrowserState>>,
    texture_store: Arc<RwLock<TextureStore>>,
    media_store: Arc<RwLock<MediaStore>>,
) {
    let window = ApplicationWindow::builder()
        .application(app)
        .title("Renderer")
        .default_width(WINDOW_WIDTH as i32)
        .default_height(WINDOW_HEIGHT as i32)
        .build();

    let dim = {
        let state = browser_state.read();
        let Some(ref tile_list) = state.tile_list else {
            log::error!("No tile list");
            return;
        };
        let d = tile_list.read().layer_list.layout_tree.root_dimension;
        d
    };

    let area = DrawingArea::new();
    area.set_content_width(dim.width as i32);
    area.set_content_height(dim.height as i32);

    {
        let bs = browser_state.clone();
        let ts = texture_store.clone();
        let ms = media_store.clone();
        area.set_draw_func(move |_area, cr, _width, _height| {
            let vis_layers = bs.read().visible_layer_list.clone();

            for (i, &visible) in vis_layers.iter().enumerate() {
                if visible {
                    do_paint(LayerId::new(i as u64), &bs);
                    do_rasterize(LayerId::new(i as u64), &bs, &ts, &ms);
                }
            }

            CairoCompositor::compose(CairoCompositorConfig {
                cr: cr.clone(),
                browser_state: bs.clone(),
                texture_store: ts.clone(),
            });
        });
    }

    let motion_controller = EventControllerMotion::new();
    let area_clone = area.clone();
    {
        let bs = browser_state.clone();
        motion_controller.connect_motion(move |_, x, y| {
            let _t_total = gosub_shared::timing_guard!("cairo_hover.total");

            let el_id = {
                let state = bs.read();
                let Some(ref tile_list) = state.tile_list else {
                    return;
                };
                let binding = tile_list.read();
                let _t = gosub_shared::timing_guard!("cairo_hover.hit_test");
                binding.layer_list.find_element_at(x, y)
            };
            let che = bs.read().current_hovered_element;

            let mut tile_ids = vec![];
            {
                let state = bs.read();
                let Some(ref tile_list) = state.tile_list else {
                    return;
                };
                match (che, el_id) {
                    (Some(current_id), Some(new_id)) if current_id != new_id => {
                        tile_list
                            .read()
                            .get_tiles_for_element(current_id)
                            .iter()
                            .for_each(|tid| tile_ids.push(*tid));
                        tile_list
                            .read()
                            .get_tiles_for_element(new_id)
                            .iter()
                            .for_each(|tid| tile_ids.push(*tid));
                    }
                    (None, Some(new_id)) => {
                        tile_list
                            .read()
                            .get_tiles_for_element(new_id)
                            .iter()
                            .for_each(|tid| tile_ids.push(*tid));
                    }
                    (Some(current_id), None) => {
                        tile_list
                            .read()
                            .get_tiles_for_element(current_id)
                            .iter()
                            .for_each(|tid| tile_ids.push(*tid));
                    }
                    _ => {}
                }
            }

            let mut state = bs.write();
            if state.current_hovered_element != el_id {
                let _t_change = gosub_shared::timing_guard!("cairo_hover.changed");
                if let Some(id) = el_id {
                    if let Some(ref tile_list) = state.tile_list {
                        let binding = tile_list.read();
                        if let Some(layout_element) = binding.layer_list.layout_tree.get_node_by_id(id) {
                            println!("Hovered element id:");
                            println!("   Layout ID : {:?}", el_id);
                            println!("   DOM ID    : {:?}", layout_element.dom_node_id);
                        }
                    }
                }

                for tile_id in &tile_ids {
                    if let Some(ref tile_list) = state.tile_list {
                        tile_list.write().invalidate_tile(*tile_id);
                    }
                }

                state.current_hovered_element = el_id;
                area_clone.queue_draw();
            }
        });
    }
    area.add_controller(motion_controller);

    let scroll = ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Always)
        .vscrollbar_policy(gtk4::PolicyType::Always)
        .child(&area)
        .build();
    window.set_child(Some(&scroll));

    connect_viewport_signals(&scroll, &area, browser_state.clone());

    let controller = gtk4::EventControllerKey::new();
    {
        let bs = browser_state.clone();
        controller.connect_key_pressed(move |_controller, keyval, _keycode, _state| {
            let mut state = bs.write();

            match keyval {
                key if key == gtk4::gdk::Key::_1 => {
                    if let Some(v) = state.visible_layer_list.get_mut(0) {
                        *v = !*v;
                        area.queue_draw();
                    }
                }
                key if key == gtk4::gdk::Key::_2 => {
                    if let Some(v) = state.visible_layer_list.get_mut(1) {
                        *v = !*v;
                        area.queue_draw();
                    }
                }
                key if key == gtk4::gdk::Key::_3 => {
                    if let Some(v) = state.visible_layer_list.get_mut(2) {
                        *v = !*v;
                        area.queue_draw();
                    }
                }
                key if key == gtk4::gdk::Key::_4 => {
                    if let Some(v) = state.visible_layer_list.get_mut(3) {
                        *v = !*v;
                        area.queue_draw();
                    }
                }
                key if key == gtk4::gdk::Key::_5 => {
                    if let Some(v) = state.visible_layer_list.get_mut(4) {
                        *v = !*v;
                        area.queue_draw();
                    }
                }
                key if key == gtk4::gdk::Key::_6 => {
                    if let Some(v) = state.visible_layer_list.get_mut(5) {
                        *v = !*v;
                        area.queue_draw();
                    }
                }
                key if key == gtk4::gdk::Key::_7 => {
                    if let Some(v) = state.visible_layer_list.get_mut(6) {
                        *v = !*v;
                        area.queue_draw();
                    }
                }
                key if key == gtk4::gdk::Key::_8 => {
                    if let Some(v) = state.visible_layer_list.get_mut(7) {
                        *v = !*v;
                        area.queue_draw();
                    }
                }
                key if key == gtk4::gdk::Key::_9 => {
                    if let Some(v) = state.visible_layer_list.get_mut(8) {
                        *v = !*v;
                        area.queue_draw();
                    }
                }
                key if key == gtk4::gdk::Key::_0 => {
                    if let Some(v) = state.visible_layer_list.get_mut(9) {
                        *v = !*v;
                        area.queue_draw();
                    }
                }
                key if key == gtk4::gdk::Key::w => {
                    match state.wireframed {
                        WireframeState::None => state.wireframed = WireframeState::Only,
                        WireframeState::Only => state.wireframed = WireframeState::Both,
                        WireframeState::Both => state.wireframed = WireframeState::None,
                    }
                    if let Some(ref tile_list) = state.tile_list {
                        tile_list.write().invalidate_all();
                    }
                    area.queue_draw();
                }
                key if key == gtk4::gdk::Key::d => {
                    state.debug_hover = !state.debug_hover;
                    if let Some(ref tile_list) = state.tile_list {
                        tile_list.write().invalidate_all();
                    }
                    area.queue_draw();
                }
                key if key == gtk4::gdk::Key::t => {
                    state.show_tilegrid = !state.show_tilegrid;
                    area.queue_draw();
                }
                key if key == gtk4::gdk::Key::s => {
                    gosub_shared::timing::TIMING_TABLE
                        .lock()
                        .print_timings(false, gosub_shared::timing::Scale::Auto);
                }
                _ => (),
            }

            glib::Propagation::Proceed
        });
    }
    window.add_controller(controller);

    window.set_default_size(WINDOW_WIDTH as i32, WINDOW_HEIGHT as i32);
    window.show();
}

fn do_paint(layer_id: LayerId, browser_state: &Arc<RwLock<BrowserState>>) {
    let state = browser_state.read();
    let Some(ref tile_list) = state.tile_list else {
        log::error!("No tile list found");
        return;
    };
    let painter = Painter::new(tile_list.read().layer_list.clone());

    let tile_ids = tile_list.read().get_intersecting_tiles(layer_id, state.viewport);
    for tile_id in tile_ids {
        let mut binding = tile_list.write();
        let Some(tile) = binding.get_tile_mut(tile_id) else {
            log::warn!("Tile not found: {:?}", tile_id);
            continue;
        };

        if tile.state == TileState::Clean || tile.state == TileState::Empty {
            continue;
        }

        for tiled_layout_element in &mut tile.elements {
            tiled_layout_element.paint_commands = painter.paint(tiled_layout_element, &state);
        }
    }
}

fn do_rasterize(
    layer_id: LayerId,
    browser_state: &Arc<RwLock<BrowserState>>,
    texture_store: &Arc<RwLock<TextureStore>>,
    media_store: &Arc<RwLock<MediaStore>>,
) {
    let state = browser_state.read();
    let mut ts = texture_store.write();
    let ms = media_store.read();

    let Some(ref tile_list) = state.tile_list else {
        log::error!("No tile list found");
        return;
    };

    let tile_ids = tile_list.read().get_intersecting_tiles(layer_id, state.viewport);
    for tile_id in tile_ids {
        let mut binding = tile_list.write();
        let Some(tile) = binding.get_tile(tile_id) else {
            log::warn!("Tile not found: {:?}", tile_id);
            continue;
        };

        if tile.state == TileState::Clean || tile.state == TileState::Empty {
            continue;
        }

        let Some(tile) = binding.get_tile_mut(tile_id) else {
            log::warn!("Tile not found: {:?}", tile_id);
            continue;
        };

        let rasterizer = CairoRasterizer::new();
        match rasterizer.rasterize(tile, &mut ts, &ms) {
            Some(texture_id) => {
                tile.texture_id = Some(texture_id);
                tile.state = TileState::Clean;
            }
            None => {
                tile.state = TileState::Empty;
            }
        }
    }
}

fn connect_viewport_signals(scroll: &ScrolledWindow, area: &DrawingArea, browser_state: Arc<RwLock<BrowserState>>) {
    let hadjustment = scroll.hadjustment();
    let vadjustment = scroll.vadjustment();

    {
        let area = area.clone();
        let vadj = vadjustment.clone();
        let bs = browser_state.clone();
        hadjustment.connect_value_changed(move |adj| {
            on_viewport_changed(&area, adj, &vadj, &bs);
        });
    }

    {
        let area = area.clone();
        let hadj = hadjustment.clone();
        let bs = browser_state.clone();
        vadjustment.connect_value_changed(move |adj| {
            on_viewport_changed(&area, &hadj, adj, &bs);
        });
    }

    {
        let area_clone = area.clone();
        let hadj = hadjustment.clone();
        let vadj = vadjustment.clone();
        let bs = browser_state.clone();
        area.connect_resize(move |_, _, _| {
            on_viewport_changed(&area_clone, &hadj, &vadj, &bs);
        });
    }
}

fn on_viewport_changed(
    area: &DrawingArea,
    hadj: &Adjustment,
    vadj: &Adjustment,
    browser_state: &Arc<RwLock<BrowserState>>,
) {
    let x = hadj.value();
    let y = vadj.value();
    let width = hadj.page_size();
    let height = vadj.page_size();

    let mut state = browser_state.write();

    if width != state.viewport.width || height != state.viewport.height {
        if let Some(ref tile_list) = state.tile_list {
            tile_list.write().invalidate_all();
        }
    }

    state.viewport = Rect::new(x, y, width, height);
    area.queue_draw();
}
