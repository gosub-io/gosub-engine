use crate::render::backend::{CompositorSink, ExternalHandle};
use gosub_shared::tab_id::TabId;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;

/// A default compositor implementation that manages frames per tab
/// and requests redraws when new frames are submitted.
pub struct DefaultCompositor {
    frames: Arc<RwLock<HashMap<TabId, ExternalHandle>>>,
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
            frames: Arc::new(RwLock::new(HashMap::new())),
            redraw_cb: Box::new(redraw_cb),
        }
    }

    fn request_redraw(&self) {
        (self.redraw_cb)();
    }

    /// Returns a cloneable handle to the frames map, allowing external code
    /// (e.g. a UI layer) to read the latest frame per tab without owning the compositor.
    pub fn frames_arc(&self) -> Arc<RwLock<HashMap<TabId, ExternalHandle>>> {
        self.frames.clone()
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
    fn submit_frame(&self, tab_id: TabId, handle: ExternalHandle) {
        self.frames.write().insert(tab_id, handle);
        self.request_redraw();
    }
}
