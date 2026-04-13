use crate::render::backend::{CompositorSink, ExternalHandle};
use crate::tab::TabId;
use std::sync::Arc;

// type SinkHandle = Arc<RwLock<dyn CompositorSink + Send>>;

/// A router/proxy that forwards compositor events to a dynamically set sink. This allows the
/// engine and its components to share a single `CompositorSink` implementation that can be changed
/// at runtime.
#[derive(Default)]
pub struct CompositorRouter {
    inner: Option<Box<dyn CompositorSink + Send>>,
}

impl CompositorRouter {
    /// Creates a new [`CompositorRouter`].
    pub fn new() -> Arc<Self> {
        Arc::new(Self { inner: None })
    }

    /// Sets the sink to which compositor events will be forwarded.
    pub fn set_sink(&mut self, sink: impl CompositorSink + 'static) {
        self.inner = Some(Box::new(sink));
    }

    /// Clears the currently set sink, if any.
    pub fn clear_sink(&mut self) -> Option<Box<dyn CompositorSink + Send>> {
        self.inner.take()
    }

    /// Calls the given closure with a reference to the currently set sink, if any.
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
