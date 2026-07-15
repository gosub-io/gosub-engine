use crate::engine::types::NavigationId;
use parking_lot::RwLock;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::Instant;

/// Things shared upwards to the zone
pub struct TabSink {
    /// When the worker thread started
    pub worker_started: OnceLock<Instant>,
    /// How many frames have been drawn
    pub frames_drawn: AtomicU64,
    /// Last reported FPS * 100 (to keep as integer)
    pub last_fps_times100: AtomicU32,
    /// Current navigation ID
    pub nav_id: RwLock<Option<NavigationId>>,
    /// Last time we painted a frame
    pub last_paint: RwLock<Option<Instant>>,
}

impl Default for TabSink {
    fn default() -> Self {
        Self::new()
    }
}

impl TabSink {
    pub fn new() -> Self {
        Self {
            worker_started: OnceLock::new(),
            frames_drawn: AtomicU64::new(0),
            last_fps_times100: AtomicU32::new(0),
            nav_id: RwLock::new(None),
            last_paint: RwLock::new(None),
        }
    }

    // ---- writers (TabWorker side) ----
    pub fn set_worker_started_now(&self) {
        let _ = self.worker_started.set(Instant::now());
    }
    pub fn inc_frame(&self) {
        self.frames_drawn.fetch_add(1, Ordering::Relaxed);
        *self.last_paint.write() = Some(Instant::now());
    }
    pub fn set_fps(&self, fps: f32) {
        let v = (fps * 100.0).clamp(0.0, u32::MAX as f32) as u32;
        self.last_fps_times100.store(v, Ordering::Relaxed);
    }
    pub fn set_nav(&self, id: NavigationId) {
        *self.nav_id.write() = Some(id);
    }
}
