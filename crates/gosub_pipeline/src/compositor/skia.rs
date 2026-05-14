use crate::common::browser_state::get_browser_state;
use crate::compositor::skia::compositor::skia_compositor;
use crate::layering::layer::LayerId;

mod compositor;

pub fn compose(canvas: &skia_safe::Canvas) {
    let binding = get_browser_state();
    let state = binding.read();

    let layers: Vec<LayerId> = state
        .visible_layer_list
        .iter()
        .enumerate()
        .filter_map(|(i, &visible)| visible.then_some(LayerId::new(i as u64)))
        .collect();

    skia_compositor(canvas, layers);
}
