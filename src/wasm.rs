use log::info;
use wasm_bindgen::prelude::wasm_bindgen;

mod css;
mod html;
mod renderer;
mod styles;
mod wgpu_context;

#[wasm_bindgen(start)]
pub fn init() {
    console_error_panic_hook::set_once();
    console_log::init().expect("could not initialize logger");

    info!("Initialized");
}
