use gosub_engine::render::backend::{CompositorSink, ExternalHandle};
use gosub_engine::tab::TabId;
use std::collections::HashMap;

/// The vello compositor is very simple. It stores the given frame through submit_frame,
/// and allows retrieval through frame_for and frame_for_mut.
pub struct VelloCompositor {
    pub frames: HashMap<TabId, ExternalHandle>,
}

impl VelloCompositor {
    #[allow(dead_code)]
    pub fn new() -> Self {
        Self { frames: HashMap::new() }
    }

    #[allow(dead_code)]
    pub fn frame_for(&self, tab_id: TabId) -> Option<&ExternalHandle> {
        self.frames.get(&tab_id)
    }

    #[allow(dead_code)]
    pub fn frame_for_mut(&mut self, tab_id: TabId) -> Option<&mut ExternalHandle> {
        self.frames.get_mut(&tab_id)
    }
}

impl CompositorSink for VelloCompositor {
    fn submit_frame(&mut self, tab_id: TabId, handle: ExternalHandle) {
        self.frames.insert(tab_id, handle);
    }
}
