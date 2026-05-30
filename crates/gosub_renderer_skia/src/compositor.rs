use crate::compositor::inner::skia_compositor;
use gosub_render_pipeline::common::browser_state::BrowserState;
use gosub_render_pipeline::common::TextureStore;
use gosub_render_pipeline::layering::layer::LayerId;
use parking_lot::RwLock;
use std::sync::Arc;

mod inner;

pub struct SkiaCompositor {}

pub fn skia_compose(
    canvas: &skia_safe::Canvas,
    browser_state: &Arc<RwLock<BrowserState>>,
    texture_store: &Arc<RwLock<TextureStore>>,
) {
    let state = browser_state.read();

    let layers: Vec<LayerId> = state
        .visible_layer_list
        .iter()
        .enumerate()
        .filter_map(|(i, &visible)| visible.then_some(LayerId::new(i as u64)))
        .collect();

    let ts = texture_store.read();
    skia_compositor(canvas, layers, &state, &ts);
}
