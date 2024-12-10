use std::sync::Arc;
use crate::CairoRenderContext;

pub struct WindowData<'a> {
    pub crc: Arc<CairoRenderContext>,
    pub _phantom: std::marker::PhantomData<&'a ()>,
}

pub struct ActiveWindowData<'a> {
    pub crc: Arc<CairoRenderContext>,
    pub surface: cairo::ImageSurface,
    pub _phantom: std::marker::PhantomData<&'a ()>,
}
