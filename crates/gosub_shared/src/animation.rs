//! Easing (timing) functions for animations.
//!
//! An [`Easing`] maps normalized progress `t ∈ [0, 1]` to an eased output. The output is
//! conventionally in `[0, 1]` too, but curves are explicitly allowed to leave that range:
//! [`Easing::Elastic`] overshoots before settling. This is the same concept as a CSS
//! `*-timing-function`, so the named curves mirror CSS and the [`Easing::CubicBezier`] variant
//! can express any of them.
//!
//! The primitive is deliberately backend-agnostic: it is consumed by scroll smoothing today and is
//! the same building block CSS transitions/animations will use later. It is `Send + Sync` so it can
//! be evaluated on a worker thread.

use std::sync::Arc;

/// Where a stepped easing places its jumps, mirroring the CSS `steps()` keywords.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepPosition {
    /// Jump at the start of each interval (CSS `jump-start` / `start`).
    JumpStart,
    /// Jump at the end of each interval (CSS `jump-end` / `end`, the default).
    JumpEnd,
    /// No jump at either end; `n` steps span `n - 1` intervals (CSS `jump-none`).
    JumpNone,
    /// Jump at both ends; `n` steps span `n + 1` intervals (CSS `jump-both`).
    JumpBoth,
}

/// A timing function mapping animation progress to an eased value.
///
/// Evaluate with [`Easing::eval`]. Input is clamped to `[0, 1]`; output may exceed it for
/// overshooting curves.
#[derive(Clone)]
pub enum Easing {
    /// Constant rate (`f(t) = t`).
    Linear,
    /// Hermite smoothstep `t²(3 − 2t)` — zero slope at both ends. This is what gosub's scroll
    /// smoothing uses; very close to [`Easing::EaseInOut`] but cheaper and exactly symmetric.
    Smoothstep,
    /// CSS `ease` — slow start, fast middle, slow end (bézier 0.25, 0.1, 0.25, 1).
    Ease,
    /// CSS `ease-in` — slow start (bézier 0.42, 0, 1, 1).
    EaseIn,
    /// CSS `ease-out` — slow end (bézier 0, 0, 0.58, 1).
    EaseOut,
    /// CSS `ease-in-out` — slow start and end (bézier 0.42, 0, 0.58, 1).
    EaseInOut,
    /// Cubic Bézier through `(0,0)`, `(x1,y1)`, `(x2,y2)`, `(1,1)`. Every CSS named curve is a
    /// specific instance of this. `x` control points are expected in `[0, 1]`.
    CubicBezier(f32, f32, f32, f32),
    /// Stepped interpolation over `n` steps (CSS `steps()`). `n` is clamped to at least 1.
    Steps(u32, StepPosition),
    /// Decelerating bounce that settles exactly at `1.0` (Penner ease-out-bounce). Stays in `[0, 1]`.
    Bounce,
    /// Overshooting, decaying oscillation that settles at `1.0` (Penner ease-out-elastic). Leaves
    /// `[0, 1]` deliberately.
    Elastic,
    /// An arbitrary curve supplied by the embedder. Must be deterministic and is expected to satisfy
    /// `f(0) ≈ 0` and `f(1) ≈ 1`; gosub does not enforce either. This is the in-process escape hatch
    /// for fully custom feels (it cannot cross a process boundary, unlike the other variants).
    Custom(Arc<dyn Fn(f32) -> f32 + Send + Sync>),
}

impl Easing {
    /// Evaluate the easing at progress `t`. `t` is clamped to `[0, 1]`; the returned value may
    /// exceed `[0, 1]` for overshooting curves (e.g. [`Easing::Elastic`]).
    pub fn eval(&self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Easing::Linear => t,
            Easing::Smoothstep => t * t * (3.0 - 2.0 * t),
            Easing::Ease => cubic_bezier(0.25, 0.1, 0.25, 1.0, t),
            Easing::EaseIn => cubic_bezier(0.42, 0.0, 1.0, 1.0, t),
            Easing::EaseOut => cubic_bezier(0.0, 0.0, 0.58, 1.0, t),
            Easing::EaseInOut => cubic_bezier(0.42, 0.0, 0.58, 1.0, t),
            Easing::CubicBezier(x1, y1, x2, y2) => cubic_bezier(*x1, *y1, *x2, *y2, t),
            Easing::Steps(n, pos) => steps(*n, *pos, t),
            Easing::Bounce => bounce(t),
            Easing::Elastic => elastic(t),
            Easing::Custom(f) => f(t),
        }
    }
}

impl std::fmt::Debug for Easing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Easing::Linear => write!(f, "Linear"),
            Easing::Smoothstep => write!(f, "Smoothstep"),
            Easing::Ease => write!(f, "Ease"),
            Easing::EaseIn => write!(f, "EaseIn"),
            Easing::EaseOut => write!(f, "EaseOut"),
            Easing::EaseInOut => write!(f, "EaseInOut"),
            Easing::CubicBezier(a, b, c, d) => write!(f, "CubicBezier({a}, {b}, {c}, {d})"),
            Easing::Steps(n, p) => write!(f, "Steps({n}, {p:?})"),
            Easing::Bounce => write!(f, "Bounce"),
            Easing::Elastic => write!(f, "Elastic"),
            Easing::Custom(_) => write!(f, "Custom(<fn>)"),
        }
    }
}

/// Evaluate a cubic Bézier easing `y` for a given `x = t` (time). The curve is parameterized by an
/// internal `s`, so we first solve `bezier_x(s) = t` (Newton–Raphson, bisection fallback) and then
/// return `bezier_y(s)`. Mirrors WebKit's `UnitBezier`.
fn cubic_bezier(x1: f32, y1: f32, x2: f32, y2: f32, t: f32) -> f32 {
    if t <= 0.0 {
        return 0.0;
    }
    if t >= 1.0 {
        return 1.0;
    }

    // Polynomial coefficients for B(s) with endpoints fixed at (0,0) and (1,1).
    let cx = 3.0 * x1;
    let bx = 3.0 * (x2 - x1) - cx;
    let ax = 1.0 - cx - bx;
    let cy = 3.0 * y1;
    let by = 3.0 * (y2 - y1) - cy;
    let ay = 1.0 - cy - by;

    let sample_x = |s: f32| ((ax * s + bx) * s + cx) * s;
    let sample_dx = |s: f32| (3.0 * ax * s + 2.0 * bx) * s + cx;
    let sample_y = |s: f32| ((ay * s + by) * s + cy) * s;

    // Newton–Raphson: fast when the derivative is well-behaved.
    let mut s = t;
    for _ in 0..8 {
        let x = sample_x(s) - t;
        if x.abs() < 1e-6 {
            return sample_y(s);
        }
        let dx = sample_dx(s);
        if dx.abs() < 1e-6 {
            break;
        }
        s = (s - x / dx).clamp(0.0, 1.0);
    }

    // Bisection fallback for the rare ill-conditioned case.
    let (mut lo, mut hi) = (0.0f32, 1.0f32);
    let mut s = t;
    for _ in 0..24 {
        let x = sample_x(s);
        if (x - t).abs() < 1e-6 {
            break;
        }
        if x < t {
            lo = s;
        } else {
            hi = s;
        }
        s = (lo + hi) * 0.5;
    }
    sample_y(s)
}

/// Stepped interpolation over `n` steps (CSS `steps()` semantics).
fn steps(n: u32, pos: StepPosition, t: f32) -> f32 {
    let n = n.max(1) as f32;
    let raw = match pos {
        StepPosition::JumpStart => (t * n).ceil(),
        StepPosition::JumpEnd | StepPosition::JumpNone | StepPosition::JumpBoth => (t * n).floor(),
    };
    let value = match pos {
        // `n` steps but `n - 1` intervals: ends touch 0 and 1.
        StepPosition::JumpNone => raw / (n - 1.0).max(1.0),
        // `n` steps but `n + 1` intervals: neither end is reached.
        StepPosition::JumpBoth => (raw + 1.0) / (n + 1.0),
        StepPosition::JumpStart | StepPosition::JumpEnd => raw / n,
    };
    value.clamp(0.0, 1.0)
}

/// Penner ease-out-bounce: decaying parabolic bounces settling at `1.0`. Stays within `[0, 1]`.
fn bounce(t: f32) -> f32 {
    const N: f32 = 7.5625;
    const D: f32 = 2.75;
    if t < 1.0 / D {
        N * t * t
    } else if t < 2.0 / D {
        let t = t - 1.5 / D;
        N * t * t + 0.75
    } else if t < 2.5 / D {
        let t = t - 2.25 / D;
        N * t * t + 0.9375
    } else {
        let t = t - 2.625 / D;
        N * t * t + 0.984_375
    }
}

/// Penner ease-out-elastic: overshooting, decaying oscillation settling at `1.0`.
fn elastic(t: f32) -> f32 {
    if t <= 0.0 {
        return 0.0;
    }
    if t >= 1.0 {
        return 1.0;
    }
    let c4 = (2.0 * std::f32::consts::PI) / 3.0;
    2.0_f32.powf(-10.0 * t) * ((t * 10.0 - 0.75) * c4).sin() + 1.0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Curves that are conventional unit easings should pin their endpoints to 0 and 1.
    #[test]
    fn endpoints_are_pinned() {
        let curves = [
            Easing::Linear,
            Easing::Smoothstep,
            Easing::Ease,
            Easing::EaseIn,
            Easing::EaseOut,
            Easing::EaseInOut,
            Easing::CubicBezier(0.42, 0.0, 0.58, 1.0),
            Easing::Bounce,
            Easing::Elastic,
        ];
        for c in &curves {
            assert!((c.eval(0.0) - 0.0).abs() < 1e-4, "{c:?} f(0) != 0");
            assert!((c.eval(1.0) - 1.0).abs() < 1e-4, "{c:?} f(1) != 1");
        }
    }

    /// Input is clamped, so out-of-range progress maps to the endpoints.
    #[test]
    fn input_is_clamped() {
        assert_eq!(Easing::Linear.eval(-5.0), 0.0);
        assert_eq!(Easing::Linear.eval(5.0), 1.0);
        assert_eq!(Easing::Smoothstep.eval(2.0), 1.0);
    }

    #[test]
    fn linear_is_identity() {
        for &t in &[0.0, 0.1, 0.25, 0.5, 0.75, 0.9, 1.0] {
            assert!((Easing::Linear.eval(t) - t).abs() < 1e-6);
        }
    }

    /// Smoothstep is symmetric about the midpoint and starts slow.
    #[test]
    fn smoothstep_symmetry_and_shape() {
        let e = Easing::Smoothstep;
        assert!((e.eval(0.5) - 0.5).abs() < 1e-6, "midpoint should be 0.5");
        // f(t) + f(1 - t) == 1 (point symmetry about (0.5, 0.5)).
        for &t in &[0.1, 0.25, 0.4] {
            assert!((e.eval(t) + e.eval(1.0 - t) - 1.0).abs() < 1e-6);
        }
        // Slow start: eased < linear in the first half.
        assert!(e.eval(0.25) < 0.25);
    }

    /// `CubicBezier(0.42, 0, 0.58, 1)` is the definition of `EaseInOut`; they must agree.
    #[test]
    fn cubic_bezier_matches_named_ease_in_out() {
        let named = Easing::EaseInOut;
        let bezier = Easing::CubicBezier(0.42, 0.0, 0.58, 1.0);
        for i in 0..=20 {
            let t = i as f32 / 20.0;
            assert!(
                (named.eval(t) - bezier.eval(t)).abs() < 1e-4,
                "mismatch at t={t}: {} vs {}",
                named.eval(t),
                bezier.eval(t)
            );
        }
    }

    /// The standard ease curves are monotonically non-decreasing.
    #[test]
    fn named_curves_are_monotonic() {
        for c in [Easing::Ease, Easing::EaseIn, Easing::EaseOut, Easing::EaseInOut, Easing::Smoothstep] {
            let mut prev = c.eval(0.0);
            for i in 1..=100 {
                let v = c.eval(i as f32 / 100.0);
                assert!(v + 1e-5 >= prev, "{c:?} not monotonic at {i}: {v} < {prev}");
                prev = v;
            }
        }
    }

    /// ease-in starts slower than ease-out (control points are mirror images).
    #[test]
    fn ease_in_vs_out() {
        assert!(Easing::EaseIn.eval(0.25) < Easing::EaseOut.eval(0.25));
    }

    #[test]
    fn custom_is_invoked() {
        let square = Easing::Custom(Arc::new(|t| t * t));
        assert!((square.eval(0.5) - 0.25).abs() < 1e-6);
        assert!((square.eval(0.0) - 0.0).abs() < 1e-6);
        assert!((square.eval(1.0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn steps_jump_end() {
        let e = Easing::Steps(4, StepPosition::JumpEnd);
        assert_eq!(e.eval(0.0), 0.0);
        assert_eq!(e.eval(0.24), 0.0);
        assert_eq!(e.eval(0.26), 0.25);
        assert_eq!(e.eval(0.99), 0.75);
        assert_eq!(e.eval(1.0), 1.0);
    }

    #[test]
    fn steps_jump_start_leaves_zero_immediately() {
        let e = Easing::Steps(4, StepPosition::JumpStart);
        assert_eq!(e.eval(0.0), 0.0);
        assert_eq!(e.eval(0.01), 0.25);
        assert_eq!(e.eval(1.0), 1.0);
    }

    /// Elastic overshoots above 1.0 somewhere in the middle before settling.
    #[test]
    fn elastic_overshoots() {
        let e = Easing::Elastic;
        let peak = (1..100).map(|i| e.eval(i as f32 / 100.0)).fold(f32::MIN, f32::max);
        assert!(peak > 1.0, "elastic should overshoot, peak was {peak}");
    }

    /// Bounce stays within the unit range.
    #[test]
    fn bounce_stays_in_range() {
        let e = Easing::Bounce;
        for i in 0..=100 {
            let v = e.eval(i as f32 / 100.0);
            assert!((-1e-4..=1.0 + 1e-4).contains(&v), "bounce left [0,1]: {v}");
        }
    }
}
