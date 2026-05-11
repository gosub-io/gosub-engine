use crate::common::browser_state::get_browser_state;
use crate::compositor::vello::compositor::vello_compositor;
use crate::compositor::Composable;
use crate::layering::layer::LayerId;

pub struct VelloCompositorConfig {}

mod compositor;

pub struct VelloCompositor {}

impl Composable for VelloCompositor {
    type Config = VelloCompositorConfig;
    type Return = vello::Scene;

    fn compose(_config: Self::Config) -> Self::Return {
        let binding = get_browser_state();
        let Ok(state) = binding.read() else {
            log::error!("Failed to acquire browser state lock, composing empty scene");
            return vello_compositor(vec![]);
        };

        let mut layers = vec![];
        for i in 0..state.visible_layer_list.len() {
            if state.visible_layer_list[i] {
                layers.push(LayerId::new(i as u64));
            }
        }

        // Compose the scene from the different layers we have selected
        vello_compositor(layers)
    }
}
