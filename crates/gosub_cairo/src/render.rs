use gosub_shared::types::Result;

pub mod window;

pub struct Renderer {
    pub context: cairo::Context,
}

impl Renderer {
    pub fn new(context: cairo::Context) -> Result<Self> {
        Ok(Self {
            context: context.clone(),
        })
    }
}
