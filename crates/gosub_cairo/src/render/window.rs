pub struct WindowData {
    pub(crate) context: cairo::Context,
}

pub struct ActiveWindowData {
    pub(crate) context: cairo::Context,
    pub(crate) surface: cairo::Surface,
}
