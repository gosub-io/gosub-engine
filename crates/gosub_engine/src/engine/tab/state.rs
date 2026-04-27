use crate::render::Viewport;
use std::time::Duration;
use url::Url;

/// State for the tab task driving a single tab.
pub(crate) struct TabRuntime {
    /// Is drawing enabled (vs suspended)
    pub drawing_enabled: bool,
    /// Target frames per second when drawing is enabled
    pub fps: u32,
    /// Interval timer for driving ticks
    pub interval: tokio::time::Interval,
    // /// Current viewport size
    // pub viewport: Viewport,
    /// Has something changed that requires a redraw
    pub dirty: bool,
    // When the last tick draw was done
    pub last_tick_draw: std::time::Instant,
}

impl Default for TabRuntime {
    fn default() -> Self {
        let fps = 60;

        Self {
            drawing_enabled: false,
            fps,
            interval: tokio::time::interval(Duration::from_secs_f64(1.0 / fps as f64)),
            dirty: false,
            last_tick_draw: std::time::Instant::now(),
        }
    }
}

/// Current state of the tab. This is a state machine that defines what the tab is doing at the moment.
#[derive(Default, Debug, Clone, PartialEq, Eq)]
#[allow(unused)]
pub enum TabState {
    /// Tab is idle (no pending network, animations, or rendering).
    #[default]
    Idle,
    /// A navigation has been requested but not started yet.
    /// The next `tick()` will transition to [`TabState::Loading`].
    PendingLoad(Url),
    /// The tab is fetching network resources (main document).
    /// When done, transitions to [`TabState::Loaded`] on success or [`TabState::Failed`] on error.
    Loading,
    /// Main document has been received and staged into the engine.
    /// The next `tick()` will begin rendering via [`TabState::PendingRendering`].
    Loaded,
    /// A render has been requested for the given viewport.
    PendingRendering(Viewport),
    /// The engine is producing a new surface for the current content.
    Rendering(Viewport),
    /// A new surface is ready for painting. The next `tick()` typically
    /// returns to [`TabState::Idle`] and sets `needs_redraw = true` in [`TickResult`].
    Rendered(Viewport),
    /// A fatal error occurred while loading or rendering.
    Failed(String),
}

/// Activity mode for a [`Tab`]. Schedulers can allocate CPU/time by mode.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[allow(unused)]
pub enum TabActivityMode {
    /// Foreground: fully active (network, layout, paint, animations ~60 Hz).
    Active,
    /// Background with animations alive but throttled (e.g., ~10 Hz).
    BackgroundLive,
    /// Background with minimal ticking (network/JS timers only, e.g., ~1 Hz).
    BackgroundIdle,
    /// Suspended: no ticking until an event or visibility change.
    Suspended,
}
