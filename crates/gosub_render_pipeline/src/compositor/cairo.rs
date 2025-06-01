use gtk4::cairo;
use crate::common::browser_state::get_browser_state;
use crate::compositor::cairo::compositor::cairo_compositor;
use crate::compositor::Composable;
use crate::layering::layer::LayerId;

pub struct CairoCompositorConfig {
    pub cr: cairo::Context,
}

mod compositor;

pub struct CairoCompositor {}

impl Composable for CairoCompositor {
    type Config = CairoCompositorConfig;
    type Return = ();

    fn compose(config: Self::Config) {

        let binding = get_browser_state();
        let state = binding.read().expect("Failed to get browser state");

        let mut layers = vec![];
        if state.visible_layer_list[0] {
            layers.push(LayerId::new(0));
        }
        if state.visible_layer_list[1] {
            layers.push(LayerId::new(1));
        }

        cairo_compositor(&config.cr, layers);
    }
}
