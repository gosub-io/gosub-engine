//! Error results that can be returned from the engine
use thiserror::Error;

/// Parser error that defines an error (message) on the given position
#[derive(Clone, Debug, PartialEq)]
pub struct ParseError {
    /// Parse error message
    pub message: String,
    /// Line number (1-based) of the error
    pub line: usize,
    // Column (1-based) on line of the error
    pub col: usize,
    // Position (0-based) of the error in the input stream
    pub offset: usize,
}

/// Serious errors and errors from third-party libraries
#[derive(Debug, Error)]
pub enum Error {
    #[error("config error: {0}")]
    Config(String),

    #[error("io error: {0}")]
    IO(#[from] std::io::Error),

    #[error("parse error: {0}")]
    Parse(String),

    #[error("utf8 conversion error: {0}")]
    Utf8(#[from] std::string::FromUtf8Error),

    #[error("test error: {0}")]
    Test(String),

    #[error("there was a problem: {0}")]
    Generic(String),
}

/// Result that can be returned which holds either T or an Error
pub type Result<T> = std::result::Result<T, anyhow::Error>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Size<T: Copy> {
    pub width: T,
    pub height: T,
}

impl<T: Copy> Size<T> {
    pub fn new(width: T, height: T) -> Self {
        Self { width, height }
    }

    pub fn uniform(size: T) -> Self {
        Self {
            width: size,
            height: size,
        }
    }

    pub fn width(&self) -> &T {
        &self.width
    }

    pub fn height(&self) -> &T {
        &self.height
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Point<T: Copy> {
    pub x: T,
    pub y: T,
}

impl<T: Copy> Point<T> {
    pub fn new(x: T, y: T) -> Self {
        Self { x, y }
    }

    pub fn x(&self) -> &T {
        &self.x
    }

    pub fn y(&self) -> &T {
        &self.y
    }
}

impl Point<u32> {
    pub const ZERO: Self = Self { x: 0, y: 0 };

    pub fn f64(&self) -> Point<f64> {
        Point::new(self.x as f64, self.y as f64)
    }

    pub fn f32(&self) -> Point<f32> {
        Point::new(self.x as f32, self.y as f32)
    }

    pub fn x32(&self) -> f32 {
        self.x as f32
    }

    pub fn y32(&self) -> f32 {
        self.y as f32
    }

    pub fn x64(&self) -> f64 {
        self.x as f64
    }

    pub fn y64(&self) -> f64 {
        self.y as f64
    }
}

impl Point<f32> {
    pub const ZERO: Self = Self { x: 0.0, y: 0.0 };

    pub fn u32(&self) -> Point<u32> {
        Point::new(self.x as u32, self.y as u32)
    }

    pub fn f64(&self) -> Point<f64> {
        Point::new(self.x as f64, self.y as f64)
    }

    pub fn x_u32(&self) -> u32 {
        self.x as u32
    }

    pub fn y_u32(&self) -> u32 {
        self.y as u32
    }

    pub fn x64(&self) -> f64 {
        self.x as f64
    }

    pub fn y64(&self) -> f64 {
        self.y as f64
    }
}

impl Point<f64> {
    pub const ZERO: Self = Self { x: 0.0, y: 0.0 };

    pub fn u32(&self) -> Point<u32> {
        Point::new(self.x as u32, self.y as u32)
    }

    pub fn f32(&self) -> Point<f32> {
        Point::new(self.x as f32, self.y as f32)
    }

    pub fn x_u32(&self) -> u32 {
        self.x as u32
    }

    pub fn y_u32(&self) -> u32 {
        self.y as u32
    }

    pub fn x32(&self) -> f32 {
        self.x as f32
    }

    pub fn y32(&self) -> f32 {
        self.y as f32
    }
}

impl Size<u32> {
    pub const ZERO: Self = Self {
        width: 0,
        height: 0,
    };

    pub fn f64(&self) -> Size<f64> {
        Size::new(self.width as f64, self.height as f64)
    }

    pub fn f32(&self) -> Size<f32> {
        Size::new(self.width as f32, self.height as f32)
    }

    pub fn w32(&self) -> f32 {
        self.width as f32
    }

    pub fn h32(&self) -> f32 {
        self.height as f32
    }

    pub fn w64(&self) -> f64 {
        self.width as f64
    }

    pub fn h64(&self) -> f64 {
        self.height as f64
    }
}

impl Size<f32> {
    pub const ZERO: Self = Self {
        width: 0.0,
        height: 0.0,
    };

    pub fn u32(&self) -> Size<u32> {
        Size::new(self.width as u32, self.height as u32)
    }

    pub fn f64(&self) -> Size<f64> {
        Size::new(self.width as f64, self.height as f64)
    }

    pub fn w_u32(&self) -> u32 {
        self.width as u32
    }

    pub fn h_u32(&self) -> u32 {
        self.height as u32
    }

    pub fn w64(&self) -> f64 {
        self.width as f64
    }

    pub fn h64(&self) -> f64 {
        self.height as f64
    }
}

impl Size<f64> {
    pub const ZERO: Self = Self {
        width: 0.0,
        height: 0.0,
    };

    pub fn u32(&self) -> Size<u32> {
        Size::new(self.width as u32, self.height as u32)
    }

    pub fn f32(&self) -> Size<f32> {
        Size::new(self.width as f32, self.height as f32)
    }

    pub fn w_u32(&self) -> u32 {
        self.width as u32
    }

    pub fn h_u32(&self) -> u32 {
        self.height as u32
    }

    pub fn w32(&self) -> f32 {
        self.width as f32
    }

    pub fn h32(&self) -> f32 {
        self.height as f32
    }
}
