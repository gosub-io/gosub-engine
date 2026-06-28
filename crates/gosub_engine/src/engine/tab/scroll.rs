//! Engine-side scroll position and its smooth-scroll animation.
//!
//! [`ScrollState`] owns the float scroll offset, the target wheel deltas accumulate toward, and the
//! per-axis [`ScrollAnimator`]s that ease between them. It is deliberately pure — it depends only on
//! the animation primitives, not on the rest of the engine — so it can be unit-tested in isolation.
//! The worker applies the integer positions it produces to the browsing context and drives [`tick`]
//! from the render loop.
//!
//! [`tick`]: ScrollState::tick

use gosub_shared::animation::{Easing, ScrollAnimator, ScrollBehavior};

/// The engine's default wheel-scroll feel. This is the single place that defines how normal
/// (non-CSS) scrolling animates; change it here to retune the global default.
pub(crate) fn default_text_scroll() -> ScrollBehavior {
    ScrollBehavior::Tween {
        duration: std::time::Duration::from_millis(220),
        easing: Easing::Smoothstep,
    }
}

/// Owns the engine's scroll offset and animates it toward a target.
pub(crate) struct ScrollState {
    /// How the offset moves toward its target. `Instant` applies moves immediately; the others ease.
    behavior: ScrollBehavior,
    /// Float current offset in CSS px. The integer the worker applies is this rounded.
    pos: (f64, f64),
    /// Goal the animation eases toward; wheel deltas accumulate here, clamped to the page bounds.
    target: (f64, f64),
    /// Per-axis animators while a smooth scroll is in flight; `None` when idle or `Instant`.
    anim: Option<(Box<dyn ScrollAnimator>, Box<dyn ScrollAnimator>)>,
}

impl ScrollState {
    pub(crate) fn new(behavior: ScrollBehavior) -> Self {
        Self {
            behavior,
            pos: (0.0, 0.0),
            target: (0.0, 0.0),
            anim: None,
        }
    }

    /// Swap the animation behavior. Any in-flight animation is dropped; the next scroll rebuilds
    /// animators from the new behavior at the current position.
    #[allow(dead_code)] // used once the engine takes over scrolling from the embedder (phase 5)
    pub(crate) fn set_behavior(&mut self, behavior: ScrollBehavior) {
        self.behavior = behavior;
        self.anim = None;
    }

    /// Accumulate a scroll delta (CSS px), clamping the target to `[0, max]` per axis.
    ///
    /// Returns `Some(pos)` — the integer offset to apply *now* — for `Instant` behavior, or `None`
    /// when the move will be animated over subsequent [`tick`](Self::tick) calls.
    pub(crate) fn scroll_by(&mut self, dx: f64, dy: f64, max_x: f64, max_y: f64) -> Option<(i32, i32)> {
        self.target.0 = (self.target.0 + dx).clamp(0.0, max_x);
        self.target.1 = (self.target.1 + dy).clamp(0.0, max_y);

        if self.behavior.is_instant() {
            self.pos = self.target;
            self.anim = None;
            return Some(round(self.pos));
        }

        // Animated: spin up per-axis animators from the current position if not already running.
        if self.anim.is_none() {
            let ax = self.behavior.make_animator(self.pos.0).expect("non-instant behavior yields an animator");
            let ay = self.behavior.make_animator(self.pos.1).expect("non-instant behavior yields an animator");
            self.anim = Some((ax, ay));
        }
        None
    }

    /// Advance an in-flight animation by `dt` seconds, returning the new integer offset while
    /// animating, or `None` when idle. Settles exactly on the target and stops animating.
    pub(crate) fn tick(&mut self, dt: f64) -> Option<(i32, i32)> {
        let (ax, ay) = self.anim.as_mut()?;
        self.pos.0 = ax.step(self.target.0, dt);
        self.pos.1 = ay.step(self.target.1, dt);
        if ax.settled() && ay.settled() {
            self.pos = self.target;
            self.anim = None;
        }
        Some(round(self.pos))
    }

    /// True while a smooth scroll is in flight (the render loop must keep ticking).
    pub(crate) fn animating(&self) -> bool {
        self.anim.is_some()
    }

    /// Jump to an exact offset, cancelling any animation (navigation / programmatic set).
    pub(crate) fn reset(&mut self, x: f64, y: f64) {
        self.pos = (x, y);
        self.target = (x, y);
        self.anim = None;
    }
}

fn round(p: (f64, f64)) -> (i32, i32) {
    (p.0.round() as i32, p.1.round() as i32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gosub_shared::animation::Easing;
    use std::time::Duration;

    fn tween(ms: u64) -> ScrollBehavior {
        ScrollBehavior::Tween {
            duration: Duration::from_millis(ms),
            easing: Easing::Linear,
        }
    }

    #[test]
    fn instant_applies_and_accumulates() {
        let mut s = ScrollState::new(ScrollBehavior::Instant);
        assert_eq!(s.scroll_by(0.0, 50.0, f64::MAX, 1000.0), Some((0, 50)));
        assert!(!s.animating());
        assert_eq!(s.scroll_by(0.0, 30.0, f64::MAX, 1000.0), Some((0, 80)));
    }

    #[test]
    fn instant_clamps_to_bounds() {
        let mut s = ScrollState::new(ScrollBehavior::Instant);
        assert_eq!(s.scroll_by(0.0, 5000.0, f64::MAX, 1000.0), Some((0, 1000)));
        assert_eq!(s.scroll_by(0.0, -9999.0, f64::MAX, 1000.0), Some((0, 0)));
    }

    #[test]
    fn animated_does_not_apply_immediately() {
        let mut s = ScrollState::new(tween(200));
        assert_eq!(s.scroll_by(0.0, 100.0, f64::MAX, 1000.0), None);
        assert!(s.animating());
    }

    #[test]
    fn animated_eases_to_target_and_settles() {
        let mut s = ScrollState::new(tween(200));
        s.scroll_by(0.0, 100.0, f64::MAX, 1000.0);
        // Linear over 200ms: 100ms → 50%, 200ms → exactly the target.
        assert_eq!(s.tick(0.1), Some((0, 50)));
        assert_eq!(s.tick(0.1), Some((0, 100)));
        assert!(!s.animating(), "settles at the end of the duration");
    }

    #[test]
    fn animated_retarget_extends_target() {
        let mut s = ScrollState::new(tween(200));
        s.scroll_by(0.0, 100.0, f64::MAX, 10_000.0);
        s.tick(0.1); // ~50, heading to 100
        assert_eq!(s.scroll_by(0.0, 100.0, f64::MAX, 10_000.0), None); // target now 200
        let mut last = (0, 0);
        for _ in 0..200 {
            if let Some(p) = s.tick(0.016) {
                last = p;
            }
            if !s.animating() {
                break;
            }
        }
        assert!(!s.animating());
        assert_eq!(last, (0, 200), "converges on the extended target");
    }

    #[test]
    fn tick_when_idle_is_none() {
        let mut s = ScrollState::new(tween(200));
        assert_eq!(s.tick(0.016), None);
    }

    #[test]
    fn reset_cancels_animation() {
        let mut s = ScrollState::new(tween(200));
        s.scroll_by(0.0, 100.0, f64::MAX, 1000.0);
        assert!(s.animating());
        s.reset(0.0, 0.0);
        assert!(!s.animating());
        assert_eq!(s.tick(0.016), None);
    }

    #[test]
    fn set_behavior_switches_to_instant() {
        let mut s = ScrollState::new(tween(200));
        s.scroll_by(0.0, 100.0, f64::MAX, 1000.0);
        assert!(s.animating());
        s.set_behavior(ScrollBehavior::Instant);
        // The next scroll applies immediately from the current (animated-so-far) position.
        assert!(s.scroll_by(0.0, 10.0, f64::MAX, 1000.0).is_some());
        assert!(!s.animating());
    }
}
