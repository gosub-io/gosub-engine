use crate::common::browser_state::BrowserState;
use crate::common::texture_store::TextureStore;
use crate::compositor::vello::compositor::vello_compositor;
use crate::compositor::Composable;
use crate::layering::layer::LayerId;
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

        let mut layers = vec![];
        for i in 0..state.visible_layer_list.len() {
            if state.visible_layer_list[i] {
                layers.push(LayerId::new(i as u64));
            }
        }

        let texture_store = config.texture_store.read();
        vello_compositor(layers, &state, &texture_store)
    }
}
