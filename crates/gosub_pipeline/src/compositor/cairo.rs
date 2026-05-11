use crate::common::browser_state::get_browser_state;
use crate::compositor::cairo::compositor::cairo_compositor;
use crate::compositor::Composable;
use crate::layering::layer::LayerId;
use gtk4::cairo;

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
        let Ok(state) = binding.read() else {
            log::error!("Failed to acquire browser state lock, skipping cairo compose");
            return;
        };

        // Invariant: visible_layer_list[i] corresponds to LayerId(i) because layers are
        // allocated sequentially from 0 in the layering engine and the list is sized to layer_count.
        let layers: Vec<LayerId> = state
            .visible_layer_list
            .iter()
            .enumerate()
            .filter_map(|(i, &visible)| if visible { Some(LayerId::new(i as u64)) } else { None })
            .collect();

        cairo_compositor(&config.cr, layers);
    }
}
