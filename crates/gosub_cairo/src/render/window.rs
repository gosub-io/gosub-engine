pub struct WindowData<'a> {
    pub context: cairo::Context,
    pub _phantom: std::marker::PhantomData<&'a ()>,
}

pub struct ActiveWindowData<'a> {
    pub context: cairo::Context,
    pub surface: cairo::ImageSurface,
    pub _phantom: std::marker::PhantomData<&'a ()>,
}
