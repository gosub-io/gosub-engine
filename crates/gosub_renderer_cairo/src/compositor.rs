use crate::compositor::compositor::cairo_compositor;
use gosub_pipeline::common::browser_state::BrowserState;
use gosub_pipeline::common::TextureStore;
use gosub_pipeline::compositor::Composable;
use gosub_pipeline::layering::layer::LayerId;
use gtk4::cairo;
use parking_lot::RwLock;
use std::sync::Arc;

pub struct CairoCompositorConfig {
    pub cr: cairo::Context,
    pub browser_state: Arc<RwLock<BrowserState>>,
    pub texture_store: Arc<RwLock<TextureStore>>,
}

mod compositor;

pub struct CairoCompositor {}

impl Composable for CairoCompositor {
    type Config = CairoCompositorConfig;
    type Return = ();

    fn compose(config: Self::Config) {
        let state = config.browser_state.read();

        let layers: Vec<LayerId> = state
            .visible_layer_list
            .iter()
            .enumerate()
            .filter_map(|(i, &visible)| if visible { Some(LayerId::new(i as u64)) } else { None })
            .collect();

        let texture_store = config.texture_store.read();
        cairo_compositor(&config.cr, layers, &state, &texture_store);
    }
}
