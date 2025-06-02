#[cfg(not(feature = "backend_cairo"))]
compile_error!("This binary can only be used with the feature 'backend_cairo' enabled");

use gtk4::glib::clone;
use gtk4::prelude::{
    AdjustmentExt, ApplicationExt, ApplicationExtManual, DrawingAreaExt, DrawingAreaExtManual,
    GtkWindowExt, WidgetExt,
};
use gtk4::{
    glib, Adjustment, Application, ApplicationWindow, DrawingArea, EventControllerMotion,
    ScrolledWindow,
};
use poc_pipeline::common;
use poc_pipeline::common::browser_state::{
    init_browser_state, BrowserState, WireframeState,
};
use poc_pipeline::common::geo::{Dimension, Rect};
use poc_pipeline::compositor::cairo::{CairoCompositor, CairoCompositorConfig};
use poc_pipeline::compositor::Composable;
use poc_pipeline::layering::layer::{LayerId, LayerList};
use poc_pipeline::layouter::taffy::TaffyLayouter;
use poc_pipeline::layouter::{CanLayout, LayoutElementId};
use poc_pipeline::painter::Painter;
use poc_pipeline::rasterizer::cairo::CairoRasterizer;
use poc_pipeline::rasterizer::Rasterable;
use poc_pipeline::rendertree_builder::RenderTree;
use poc_pipeline::tiler::{TileList, TileState};
use std::sync::RwLock;
use gosub_render_pipeline::common::browser_state::{with_browser_state, BrowserState};
use gosub_render_pipeline::layering::layer::LayerId;
use gosub_render_pipeline::{with_browser_state, with_browser_state_mut};

const TILE_DIMENSION: f64 = 256.0;

const WINDOW_WIDTH: f64 = 1024.0;
const WINDOW_HEIGHT: f64 = 768.0;

fn main() {
    // --------------------------------------------------------------------
    // Generate a DOM tree
    // let doc = common::document::create_document();
    // let doc = common::document::parser::document_from_json("tables.json");
    // let doc = common::document::parser::document_from_json("news.ycombinator.com.json");
    let doc = common::document::parser::document_from_json("cm.json");
    let mut output = String::new();
    doc.print_tree(&mut output).expect("");
    println!("{}", output);

    // --------------------------------------------------------------------
    // Convert the DOM tree into a render-tree that has all the non-visible elements removed
    let mut render_tree = RenderTree::new(doc);
    render_tree.parse();
    // render_tree.print();

    // --------------------------------------------------------------------
    // Layout the render-tree into a layout-tree
    let mut layouter = TaffyLayouter::new();
    let layout_tree = layouter.layout(render_tree, None);
    layouter.print_tree();
    println!(
        "Layout width: {}, height: {}",
        layout_tree.root_dimension.width, layout_tree.root_dimension.height
    );

    // -------------------------------------------------------------------  -
    // Generate render layers
    let layer_list = LayerList::new(layout_tree);
    // for (layer_id, layer) in layer_list.layers.read().expect("").iter() {
    //     println!("Layer: {} (order: {})", layer_id, layer.order);
    //     for element in layer.elements.iter() {
    //         println!("  Element: {}", element);
    //     }
    // }

    // --------------------------------------------------------------------
    // Tiling phase
    let mut tile_list = TileList::new(layer_list, Dimension::new(TILE_DIMENSION, TILE_DIMENSION));
    tile_list.generate();
    // tile_list.print_list();

    // --------------------------------------------------------------------
    // At this point, we have done everything we can before painting. The rest
    // is completed in the draw function of the UI.

    // Render the layout-tree into a GTK window
    let app = Application::builder()
        .application_id("io.gosub.renderer")
        .build();

    let browser_state = BrowserState {
        visible_layer_list: vec![true; 10],
        wireframed: WireframeState::None,
        debug_hover: false,
        current_hovered_element: None,
        tile_list: RwLock::new(tile_list),
        show_tilegrid: true,
        viewport: Rect::ZERO,
    };
    init_browser_state(browser_state);

    app.connect_activate(move |app| {
        build_ui(app);
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

"
    );

    app.run();
}

fn build_ui(app: &Application) {
    let window = ApplicationWindow::builder()
        .application(app)
        .title("Renderer")
        .default_width(WINDOW_WIDTH as i32)
        .default_height(WINDOW_HEIGHT as i32)
        .build();

    // Find the root layout dimension so we can set the viewport correctly
    let dim = with_browser_state!(C, state => {
        state.tile_list
            .read()
            .unwrap()
            .layer_list
            .layout_tree
            .clone()
            .root_dimension
            .clone();
    });

    let area = DrawingArea::new();
    area.set_content_width(dim.width as i32);
    area.set_content_height(dim.height as i32);
    area.set_draw_func(move |_area, cr, _width, _height| {

        let vis_layers = with_browser_state!(C, state => {
            state.visible_layer_list.clone()
        });

        if vis_layers[0] {
            do_paint(LayerId::new(0));
            do_rasterize(LayerId::new(0));
        }
        if vis_layers[1] {
            do_paint(LayerId::new(1));
            do_rasterize(LayerId::new(1));
        }

        CairoCompositor::compose(CairoCompositorConfig { cr: cr.clone() });
    });

    // When we move the mouse, we can detect which element is currently hovered upon
    // This allows us to trigger events (OnElementLeave, onElementEnter). At that point,
    // we trigger a redraw, since there can be things that need to be updated.
    let motion_controller = EventControllerMotion::new();
    let area_clone = area.clone();
    motion_controller.connect_motion(move |_, x, y| {

        let (che, el_id) = with_browser_state!(C, state => {
            let el_id = state
                .tile_list
                .read()
                .unwrap()
                .layer_list
                .find_element_at(x, y)
                .clone();
            let che = state.current_hovered_element.clone();

            (che, el_id)
        });

        let mut tile_ids = vec![];
        match (che, el_id) {
            (Some(current_id), Some(new_id)) if current_id != new_id => {
                with_browser_state_mut!(C, state => {
                    state
                        .tile_list
                        .read()
                        .unwrap()
                        .get_tiles_for_element(current_id)
                        .iter()
                        .for_each(|tile_id| {
                            tile_ids.push(*tile_id);
                        });
                });
                with_browser_state_mut!(C, state => {
                    state
                        .tile_list
                        .read()
                        .unwrap()
                        .get_tiles_for_element(new_id)
                        .iter()
                        .for_each(|tile_id| {
                            tile_ids.push(*tile_id);
                        });
                });
            }
            (None, Some(new_id)) => {
                let mut tile_ids = vec![];
                with_browser_state_mut!(C, state => {
                    state
                        .tile_list
                        .read()
                        .unwrap()
                        .get_tiles_for_element(new_id)
                        .iter()
                        .for_each(|tile_id| {
                            tile_ids.push(*tile_id);
                        });
                });
            }
            (Some(current_id), None) => {
                let mut tile_ids = vec![];
                with_browser_state_mut!(C, state => {
                    state
                        .tile_list
                        .read()
                        .unwrap()
                        .get_tiles_for_element(current_id)
                        .iter()
                        .for_each(|tile_id| {
                            tile_ids.push(*tile_id);
                        });
                    });
            }
            _ => {}
        }


        with_browser_state_mut!(C, state => {
            if state.current_hovered_element != el_id {
                if el_id.is_some() {
                    let binding = state.tile_list.read().unwrap();
                    let layout_element = binding
                        .layer_list
                        .layout_tree
                        .get_node_by_id(el_id.unwrap())
                        .unwrap();
                    println!("Hovered element id:");
                    println!("   Layout ID : {:?}", el_id);
                    println!("   DOM ID    : {:?}", layout_element.dom_node_id);
                    drop(binding);
                }

                for tile_id in &tile_ids {
                    // It's ok when we have double tiles in the list. We just set the tile to dirty again.
                    state.tile_list.write().unwrap().invalidate_tile(*tile_id);
                }

                state.current_hovered_element = el_id;
                area_clone.queue_draw();
            }
        });
    });
    area.add_controller(motion_controller);

    let scroll = ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Always)
        .vscrollbar_policy(gtk4::PolicyType::Always)
        .child(&area)
        .build();
    window.set_child(Some(&scroll));

    connect_viewport_signals(&scroll, &area);

    // Add keyboard shortcuts to trigger some of the rendering options
    let controller = gtk4::EventControllerKey::new();
    controller.connect_key_pressed(move |_controller, keyval, _keycode, _state| {
        with_browser_state_mut!(C, state => {

            match keyval {
                // numeric keys triggers the visibility of the layers
                key if key == gtk4::gdk::Key::_1 => {
                    state.visible_layer_list[0] = !state.visible_layer_list[0];
                    area.queue_draw();
                }
                key if key == gtk4::gdk::Key::_2 => {
                    state.visible_layer_list[1] = !state.visible_layer_list[1];
                    area.queue_draw();
                }
                key if key == gtk4::gdk::Key::_3 => {
                    state.visible_layer_list[2] = !state.visible_layer_list[2];
                    area.queue_draw();
                }
                key if key == gtk4::gdk::Key::_4 => {
                    state.visible_layer_list[3] = !state.visible_layer_list[3];
                    area.queue_draw();
                }
                key if key == gtk4::gdk::Key::_5 => {
                    state.visible_layer_list[4] = !state.visible_layer_list[4];
                    area.queue_draw();
                }
                key if key == gtk4::gdk::Key::_6 => {
                    state.visible_layer_list[5] = !state.visible_layer_list[5];
                    area.queue_draw();
                }
                key if key == gtk4::gdk::Key::_7 => {
                    state.visible_layer_list[6] = !state.visible_layer_list[6];
                    area.queue_draw();
                }
                key if key == gtk4::gdk::Key::_8 => {
                    state.visible_layer_list[7] = !state.visible_layer_list[7];
                    area.queue_draw();
                }
                key if key == gtk4::gdk::Key::_9 => {
                    state.visible_layer_list[8] = !state.visible_layer_list[8];
                    area.queue_draw();
                }
                key if key == gtk4::gdk::Key::_0 => {
                    state.visible_layer_list[9] = !state.visible_layer_list[9];
                    area.queue_draw();
                }
                // toggle wireframed elements
                key if key == gtk4::gdk::Key::w => {
                    match state.wireframed {
                        WireframeState::None => state.wireframed = WireframeState::Only,
                        WireframeState::Only => state.wireframed = WireframeState::Both,
                        WireframeState::Both => state.wireframed = WireframeState::None,
                    }
                    state
                        .tile_list
                        .write()
                        .expect("Failed to get tile list")
                        .invalidate_all();
                    area.queue_draw();
                }
                // toggle displaying only the hovered element
                key if key == gtk4::gdk::Key::d => {
                    state.debug_hover = !state.debug_hover;
                    state
                        .tile_list
                        .write()
                        .expect("Failed to get tile list")
                        .invalidate_all();
                    area.queue_draw();
                }
                // toggle tile grid
                key if key == gtk4::gdk::Key::t => {
                    state.show_tilegrid = !state.show_tilegrid;
                    area.queue_draw();
                }
                _ => (),
            }
        });
        glib::Propagation::Proceed
    });
    window.add_controller(controller);

    window.set_default_size(WINDOW_WIDTH as i32, WINDOW_HEIGHT as i32);
    window.show();
}

/// Paint all the dirty tiles that are in view
fn do_paint(layer_id: LayerId) {
    with_browser_state!(C, state => {
        let painter = Painter::new(state.tile_list.read().unwrap().layer_list.clone());

        let tile_ids = state
            .tile_list
            .read()
            .unwrap()
            .get_intersecting_tiles(layer_id, state.viewport);
        for tile_id in tile_ids {
            // get tile
            let mut binding = state.tile_list.write().expect("Failed to get tile list");
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
    });
}

fn do_rasterize(layer_id: LayerId) {
    with_browser_state!(C, state => {

        let tile_ids = state
            .tile_list
            .read()
            .unwrap()
            .get_intersecting_tiles(layer_id, state.viewport);
        for tile_id in tile_ids {
            // get tile
            let mut binding = state.tile_list.write().expect("Failed to get tile list");
            let Some(tile) = binding.get_tile(tile_id) else {
                log::warn!("Tile not found: {:?}", tile_id);
                continue;
            };

            // if not dirty, no need to render and continue
            if tile.state == TileState::Clean || tile.state == TileState::Empty {
                continue;
            }

            // Rasterize the tile into a texture
            // println!("Generating painting commands for tile");
            let Some(tile) = binding.get_tile_mut(tile_id) else {
                log::warn!("Tile not found: {:?}", tile_id);
                continue;
            };

            let rasterizer = CairoRasterizer::new();
            match rasterizer.rasterize(tile) {
                Some(texture_id) => {
                    tile.texture_id = Some(texture_id);
                    tile.state = TileState::Clean;
                }
                None => {
                    tile.state = TileState::Empty;
                }
            }
        }
    });
}

// Function to set up viewport event listeners
fn connect_viewport_signals(scroll: &ScrolledWindow, area: &DrawingArea) {
    let hadjustment = scroll.hadjustment();
    let vadjustment = scroll.vadjustment();

    // Connect to the scroll changes
    hadjustment.connect_value_changed(clone!(
        #[weak]
        area,
        #[weak]
        vadjustment,
        move |adj| {
            on_viewport_changed(&area, adj, &vadjustment);
        }
    ));

    vadjustment.connect_value_changed(clone!(
        #[weak]
        area,
        #[weak]
        hadjustment,
        move |adj| {
            on_viewport_changed(&area, &hadjustment, adj);
        }
    ));

    // Connect to the DrawingArea's resize signal
    area.connect_resize(clone!(
        #[weak]
        area,
        #[weak]
        hadjustment,
        #[weak]
        vadjustment,
        move |_, _, _| {
            on_viewport_changed(&area, &hadjustment, &vadjustment);
        }
    ));
}

// Function to handle viewport changes
fn on_viewport_changed(area: &DrawingArea, hadj: &Adjustment, vadj: &Adjustment) {
    let x = hadj.value(); // Scroll offset X
    let y = vadj.value(); // Scroll offset Y
    let width = hadj.page_size(); // Visible width
    let height = vadj.page_size(); // Visible height

    println!(
        "Visible viewport: x={} y={} width={} height={}",
        x, y, width, height
    );

    with_browser_state_mut!(C, state => {
        // If we changed the viewport size, we need to invalidate all tiles
        if width != state.viewport.width || height != state.viewport.height {
            state
                .tile_list
                .write()
                .expect("Failed to get tile list")
                .invalidate_all();
        }

        state.viewport = Rect::new(x, y, width, height);
    });

    area.queue_draw();
}
