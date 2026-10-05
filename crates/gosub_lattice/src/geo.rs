//! Minimal geometry types used by the table layout algorithm.
//!
//! Lattice only needs a 2D point and a 2D size in `f64`, so it carries its own tiny
//! definitions rather than depending on a larger shared geometry crate. This keeps the
//! crate freestanding (its only dependency is `anyhow`).

/// A 2D point in `f64` space.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

impl Point {
    pub const ZERO: Self = Self { x: 0.0, y: 0.0 };

    pub fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}

/// A 2D size in `f64` space.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Size {
    pub width: f64,
    pub height: f64,
}

impl Size {
    pub const ZERO: Self = Self {
        width: 0.0,
        height: 0.0,
    };

    pub fn new(width: f64, height: f64) -> Self {
        Self { width, height }
    }
}
