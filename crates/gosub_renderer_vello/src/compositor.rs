use crate::compositor::compositor::vello_compositor;
use gosub_pipeline::common::browser_state::BrowserState;
use gosub_pipeline::common::TextureStore;
use gosub_pipeline::compositor::Composable;
use gosub_pipeline::layering::layer::LayerId;
use parking_lot::RwLock;
use std::sync::Arc;

pub struct VelloCompositorConfig {
    pub browser_state: Arc<RwLock<BrowserState>>,
    pub texture_store: Arc<RwLock<TextureStore>>,
}

mod compositor;

pub struct VelloCompositor {}

impl Composable for VelloCompositor {
    type Config = VelloCompositorConfig;
    type Return = vello::Scene;

    fn compose(config: Self::Config) -> Self::Return {
        let state = config.browser_state.read();

        let layers: Vec<LayerId> = state
            .visible_layer_list
            .iter()
            .enumerate()
            .filter_map(|(i, &visible)| if visible { Some(LayerId::new(i as u64)) } else { None })
            .collect();

        let texture_store = config.texture_store.read();
        vello_compositor(layers, &state, &texture_store)
    }
}
