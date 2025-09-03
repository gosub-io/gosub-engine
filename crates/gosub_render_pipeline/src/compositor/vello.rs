use crate::common::render_state::RenderState;
use crate::compositor::Composable;
use crate::compositor::vello::compositor::vello_compositor;
use crate::layering::layer::LayerId;
use crate::with_render_state;

pub struct VelloCompositorConfig {}

mod compositor;

pub struct VelloCompositor {}

impl Composable for VelloCompositor {
    type Config = VelloCompositorConfig;
    type Return = vello::Scene;

    fn compose(_config: Self::Config) -> Self::Return {
        with_render_state!(C, state => {
            let mut layers = vec![];
            for i in 0..state.visible_layer_list.len() {
                if state.visible_layer_list[i] {
                    layers.push(LayerId::new(i as u64));
                }
            }

            // Compose the scene from the different layers we have selected
            vello_compositor(layers)
        });
    }
}

