use crate::Scene;

pub struct WindowData {
    pub cr: Option<cairo::Context>,
    pub scene: Scene,
}

pub struct ActiveWindowData {
    pub cr: cairo::Context,
}
