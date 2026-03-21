use crate::common::browser_state::get_browser_state;
use crate::compositor::Composable;
use crate::compositor::skia::compositor::skia_compositor;
use crate::layering::layer::LayerId;

pub struct SkiaCompositorConfig<'a> {
    pub canvas: &'a skia_safe::Canvas,
}

mod compositor;

pub struct SkiaCompositor<'a> {
    _marker: std::marker::PhantomData<&'a ()>,
}

impl<'a> Composable for SkiaCompositor<'a> {
    type Config = SkiaCompositorConfig<'a>;
    type Return = ();

    fn compose(config: Self::Config) -> Self::Return {
        let binding = get_browser_state();
        let state = binding.read().expect("Failed to get browser state");

        let mut layers = vec![];
        for i in 0..state.visible_layer_list.len() {
            if state.visible_layer_list[i] {
                layers.push(LayerId::new(i as u64));
            }
        }

        // Compose the scene from the different layers we have selected
        skia_compositor(config.canvas, layers);
    }
}

