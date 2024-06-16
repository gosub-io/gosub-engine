use std::sync::Arc;

use vello::Renderer;

use crate::Scene;

use super::{InstanceAdapter, SurfaceWrapper};

pub struct WindowData {
    pub(crate) adapter: Arc<InstanceAdapter>,
    pub(crate) renderer: Renderer,
    pub(crate) scene: Scene,
}

pub struct ActiveWindowData<'a> {
    pub(crate) surface: SurfaceWrapper<'a>,
}
