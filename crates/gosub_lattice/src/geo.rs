//! Minimal geometry types used by the table layout algorithm.
//!
//! Lattice only needs a 2D point and a 2D size in `f32`, so it carries its own tiny
//! definitions rather than depending on a larger shared geometry crate. This keeps the
//! crate freestanding (its only dependencies are `anyhow` and `log`).

/// A 2D point in `f32` space.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

impl Point {
    pub const ZERO: Self = Self { x: 0.0, y: 0.0 };

    pub fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

/// A 2D size in `f32` space.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Size {
    pub width: f32,
    pub height: f32,
}

impl Size {
    pub const ZERO: Self = Self {
        width: 0.0,
        height: 0.0,
    };

    pub fn new(width: f32, height: f32) -> Self {
        Self { width, height }
    }
}
