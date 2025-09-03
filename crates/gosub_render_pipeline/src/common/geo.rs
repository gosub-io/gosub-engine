/// A simple rectangle with a position (x, y) and dimensions (width, height).
#[derive(Debug, Clone, Copy)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    pub const ZERO : Rect = Rect { x: 0.0, y: 0.0, width: 0.0, height: 0.0 };

    /// Create a new rectangle with the given position and dimensions.
    pub fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self { x, y, width, height }
    }

    /// Returns the dimension of the rectangle.
    pub fn dimension(&self) -> Dimension {
        Dimension::new(self.width, self.height)
    }

    /// Converts a size and dimension into a rectangle.
    #[allow(unused)]
    pub fn from_coord_dimension(coord: Coordinate, dimension: Dimension) -> Self {
        Self {
            x: coord.x,
            y: coord.y,
            width: dimension.width,
            height: dimension.height,
        }
    }

    /// Returns a new rect that is shifted by the given coordinate.
    pub fn shift(&self, coord: Coordinate) -> Self {
        Self {
            x: self.x + coord.x,
            y: self.y + coord.y,
            width: self.width,
            height: self.height,
        }
    }
}

impl Into<Coordinate> for Rect {
    fn into(self) -> Coordinate {
        Coordinate::new(self.x, self.y)
    }
}

impl Into<Dimension> for Rect {
    fn into(self) -> Dimension {
        Dimension::new(self.width, self.height)
    }
}


/// A coordinate is an X/Y position. Could be negative if needed.
#[allow(unused)]
#[derive(Clone, Copy, Debug)]
pub struct Coordinate {
    pub x: f64,
    pub y: f64,
}

impl Coordinate {
    pub const ZERO : Coordinate = Coordinate { x: 0.0, y: 0.0 };

    pub fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}

/// Dimension in width and height. Together with a Dimension it forms a Rect.
#[allow(unused)]
#[derive(Clone, Debug, Copy, PartialEq)]
pub struct Dimension {
    pub width: f64,
    pub height: f64,
}

impl Dimension {
    pub const ZERO : Dimension = Dimension { width: 0.0, height: 0.0 };

    #[allow(unused)]
    pub fn new(width: f64, height: f64) -> Self {
        Self { width, height }
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rect_from_coord_dimension() {
        let coord = Coordinate::new(5.0, 5.0);
        let dimension = Dimension::new(10.0, 10.0);
        let rect = Rect::from_coord_dimension(coord, dimension);
        assert_eq!(rect.x, 5.0);
        assert_eq!(rect.y, 5.0);
        assert_eq!(rect.width, 10.0);
        assert_eq!(rect.height, 10.0);
    }

    #[test]
    fn test_into_coordinate() {
        let rect = Rect::new(10.0, 20.0, 0.0, 0.0);
        let coord: Coordinate = rect.into();
        assert_eq!(coord.x, 10.0);
        assert_eq!(coord.y, 20.0);
    }

    #[test]
    fn test_into_dimension() {
        let rect = Rect::new(0.0, 0.0, 10.0, 10.0);
        let dimension: Dimension = rect.into();
        assert_eq!(dimension.width, 10.0);
        assert_eq!(dimension.height, 10.0);
    }

    #[test]
    fn test_coordinate_new() {
        let coord = Coordinate::new(10.0, 20.0);
        assert_eq!(coord.x, 10.0);
        assert_eq!(coord.y, 20.0);
    }

    #[test]
    fn test_dimension_new() {
        let dimension = Dimension::new(10.0, 20.0);
        assert_eq!(dimension.width, 10.0);
        assert_eq!(dimension.height, 20.0);
    }
}