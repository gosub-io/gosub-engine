/// Rectangular dimensions commonly used for certain properties such as margin/padding
#[derive(Debug, PartialEq, Clone)]
#[repr(C)]
pub struct Rectangle {
    pub top: f64,
    pub left: f64,
    pub right: f64,
    pub bottom: f64,
}

impl Default for Rectangle {
    fn default() -> Self {
        Self::new()
    }
}

impl Rectangle {
    #[must_use]
    pub fn new() -> Self {
        Self {
            top: 0.,
            left: 0.,
            right: 0.,
            bottom: 0.,
        }
    }

    #[must_use]
    pub fn with_values(top: f64, left: f64, right: f64, bottom: f64) -> Self {
        Self {
            top,
            left,
            right,
            bottom,
        }
    }
}

/// The position of the render cursor used to determine where
/// to draw an object
#[derive(Debug, PartialEq, Clone)]
#[repr(C)]
pub struct Position {
    pub x: f64,
    pub y: f64,
}

impl Position {
    #[must_use]
    pub fn new() -> Self {
        Self { x: 0., y: 0. }
    }

    #[must_use]
    pub fn new_from_existing(position: &Self) -> Self {
        Self {
            x: position.x,
            y: position.y,
        }
    }

    /// Move position to (x, y)
    pub fn move_to(&mut self, x: f64, y: f64) {
        self.x = x;
        self.y = y;
    }

    /// Move position relative to another position.
    /// x = relative.x + `x_offset`
    /// y = relative.y + `y_offset`
    pub fn move_relative_to(&mut self, relative_position: &Self, x_offset: f64, y_offset: f64) {
        self.x = relative_position.x + x_offset;
        self.y = relative_position.y + y_offset;
    }

    /// Adjust y by an offset.
    /// y += `offset_y`
    pub fn offset_y(&mut self, offset_y: f64) {
        self.y += offset_y;
    }

    /// Adjust x by an offset.
    /// x += `offset_x`
    pub fn offset_x(&mut self, offset_x: f64) {
        self.x += offset_x;
    }
}

impl Default for Position {
    fn default() -> Self {
        Self::new()
    }
}
