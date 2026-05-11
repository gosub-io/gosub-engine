use crate::engine::types::NavigationId;
use parking_lot::RwLock;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::Instant;
use url::Url;

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
    /// Current URL
    pub current_url: RwLock<Option<Url>>,
    /// Last time we painted a frame
    pub last_paint: RwLock<Option<Instant>>,
    /// Raw HTML source of the current document, set after each document load
    pub source_html: RwLock<Option<String>>,
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
            current_url: RwLock::new(None),
            last_paint: RwLock::new(None),
            source_html: RwLock::new(None),
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
    pub fn set_current_url(&self, url: Url) {
        *self.current_url.write() = Some(url);
    }
    pub fn set_source_html(&self, html: String) {
        *self.source_html.write() = Some(html);
    }

    pub fn snapshot(&self) -> TabMetricsSnapshot {
        TabMetricsSnapshot {
            worker_started: self.worker_started.get().copied(),
            frames_drawn: self.frames_drawn.load(Ordering::Relaxed),
            last_fps: self.last_fps_times100.load(Ordering::Relaxed) as f32 / 100.0,
            nav_id: *self.nav_id.read(),
            current_url: self.current_url.read().clone(),
            last_paint: *self.last_paint.read(),
        }
    }
}

/// Snapshot of the metrics to return to the caller. This removes all the locks and atomics
/// for easier extraction.
#[derive(Debug, Clone)]
pub struct TabMetricsSnapshot {
    pub worker_started: Option<Instant>,
    pub frames_drawn: u64,
    pub last_fps: f32,
    pub nav_id: Option<NavigationId>,
    pub current_url: Option<Url>,
    pub last_paint: Option<Instant>,
}
