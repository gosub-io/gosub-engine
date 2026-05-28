use crate::render::backend::{CompositorSink, ExternalHandle};
use gosub_shared::tab_id::TabId;
use parking_lot::RwLock;
use std::collections::HashMap;

/// A default compositor implementation that manages frames per tab
/// and requests redraws when new frames are submitted.
pub struct DefaultCompositor {
    pub frames: RwLock<HashMap<TabId, ExternalHandle>>,
    redraw_cb: Box<dyn Fn() + Send + Sync + 'static>,
}

impl Default for DefaultCompositor {
    fn default() -> Self {
        Self::new(|| {})
    }
}

impl DefaultCompositor {
    pub fn new<F: Fn() + Send + Sync + 'static>(redraw_cb: F) -> Self {
        Self {
            frames: RwLock::new(HashMap::new()),
            redraw_cb: Box::new(redraw_cb),
        }
    }

    fn request_redraw(&self) {
        (self.redraw_cb)();
    }

    pub fn frame_for(&self, tab_id: TabId) -> Option<ExternalHandle> {
        self.frames.read().get(&tab_id).cloned()
    }

    pub fn frame_for_mut(&self, tab_id: TabId, f: impl FnOnce(&mut ExternalHandle)) {
        if let Some(h) = self.frames.write().get_mut(&tab_id) {
            f(h);
        }
    }
}

impl CompositorSink for DefaultCompositor {
    fn submit_frame(&mut self, tab_id: TabId, handle: ExternalHandle) {
        self.frames.write().insert(tab_id, handle);
        self.request_redraw();
    }
}
