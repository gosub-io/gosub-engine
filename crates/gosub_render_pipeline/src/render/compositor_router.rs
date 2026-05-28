use crate::render::backend::{CompositorSink, ExternalHandle};
use gosub_shared::tab_id::TabId;
use std::sync::Arc;

/// A router/proxy that forwards compositor events to a dynamically set sink.
#[derive(Default)]
pub struct CompositorRouter {
    inner: Option<Box<dyn CompositorSink + Send>>,
}

impl CompositorRouter {
    pub fn new() -> Arc<Self> {
        Arc::new(Self { inner: None })
    }

    #[allow(clippy::implied_bounds_in_impls)]
    pub fn set_sink(&mut self, sink: impl CompositorSink + Send + 'static) {
        self.inner = Some(Box::new(sink));
    }

    pub fn clear_sink(&mut self) -> Option<Box<dyn CompositorSink + Send>> {
        self.inner.take()
    }

    #[inline]
    fn with_sink<F>(&mut self, f: F)
    where
        F: FnOnce(&mut dyn CompositorSink),
    {
        if let Some(sink) = self.inner.as_deref_mut() {
            f(sink);
        }
    }
}

impl CompositorSink for CompositorRouter {
    fn submit_frame(&mut self, tab_id: TabId, handle: ExternalHandle) {
        self.with_sink(|sink| sink.submit_frame(tab_id, handle));
    }
}
