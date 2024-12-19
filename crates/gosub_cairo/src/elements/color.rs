use gosub_shared::render_backend::Color as TColor;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GsColor {
    pub r: f64,
    pub g: f64,
    pub b: f64,
    pub a: f64,
}

impl GsColor {
    pub const fn rgba64(r: f64, g: f64, b: f64, a: f64) -> Self {
        Self { r, g, b, a }
    }

    pub const fn rgba32(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self {
            r: r as f64,
            g: g as f64,
            b: b as f64,
            a: a as f64,
        }
    }

    pub const fn rgba8(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self {
            r: r as f64 / 255.0,
            g: g as f64 / 255.0,
            b: b as f64 / 255.0,
            a: a as f64 / 255.0,
        }
    }

    #[inline]
    pub fn r8(&self) -> u8 {
        (self.r * 255.0) as u8
    }

    #[inline]
    pub fn g8(&self) -> u8 {
        (self.g * 255.0) as u8
    }

    #[inline]
    pub fn b8(&self) -> u8 {
        (self.b * 255.0) as u8
    }

    #[inline]
    pub fn a8(&self) -> u8 {
        (self.a * 255.0) as u8
    }
}

impl TColor for GsColor {
    fn with_alpha(r: u8, g: u8, b: u8, a: u8) -> Self {
        GsColor::rgba8(r, g, b, a)
    }

    fn r(&self) -> u8 {
        self.r8()
    }

    fn g(&self) -> u8 {
        self.g8()
    }

    fn b(&self) -> u8 {
        self.b8()
    }

    fn a(&self) -> u8 {
        self.a8()
    }

    const WHITE: Self = GsColor::rgba8(255, 255, 255, 255);
    const BLACK: Self = GsColor::rgba8(0, 0, 0, 255);
    const RED: Self = GsColor::rgba8(255, 0, 0, 255);
    const GREEN: Self = GsColor::rgba8(0, 255, 0, 255);
    const BLUE: Self = GsColor::rgba8(0, 0, 255, 255);
    const YELLOW: Self = GsColor::rgba8(255, 255, 0, 255);
    const CYAN: Self = GsColor::rgba8(0, 255, 255, 255);
    const MAGENTA: Self = GsColor::rgba8(255, 0, 255, 255);
    const TRANSPARENT: Self = GsColor::rgba8(255, 255, 255, 0);
}
