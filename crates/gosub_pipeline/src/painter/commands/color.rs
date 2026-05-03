use csscolorparser::Color as ccpColor;

/// Our colors are internally f32 (0.0 to 1.0) but we can convert them to u8 (0 to 255) with r8, g8, b8, a8
/// It also allows creating colors by css name
#[derive(Clone, Debug)]
pub struct Color {
    r: f32,
    g: f32,
    b: f32,
    a: f32,
}

impl Color {
    pub const TRANSPARENT: Color = Color { r: 0.0, g: 0.0, b: 0.0, a: 0.0 };
    pub const WHITE: Color = Color { r: 1.0, g: 1.0, b: 1.0, a: 1.0 };
    pub const BLACK: Color = Color { r: 0.0, g: 0.0, b: 0.0, a: 1.0 };
    pub const RED: Color = Color { r: 1.0, g: 0.0, b: 0.0, a: 1.0 };
    pub const GREEN: Color = Color { r: 0.0, g: 1.0, b: 0.0, a: 1.0 };
    pub const BLUE: Color = Color { r: 0.0, g: 0.0, b: 1.0, a: 1.0 };
    pub const CYAN: Color = Color { r: 0.0, g: 1.0, b: 1.0, a: 1.0 };
    pub const MAGENTA: Color = Color { r: 1.0, g: 0.0, b: 1.0, a: 1.0 };
    pub const YELLOW: Color = Color { r: 1.0, g: 1.0, b: 0.0, a: 1.0 };

    pub fn from_rgb(r: f32, g: f32, b: f32) -> Self {
        Color { r, g, b, a: 1.0 }
    }

    pub fn from_rgba(r: f32, g: f32, b: f32, a: f32) -> Self {
        Color { r, g, b, a }
    }

    pub fn from_rgb8(r: u8, g: u8, b: u8) -> Self {
        Color {
            r: r as f32 / 255.0,
            g: g as f32 / 255.0,
            b: b as f32 / 255.0,
            a: 1.0
        }
    }

    pub fn from_rgba8(r: u8, g: u8, b: u8, a: u8) -> Self {
        Color {
            r : r as f32 / 255.0,
            g : g as f32 / 255.0,
            b : b as f32 / 255.0,
            a : a as f32 / 255.0
        }
    }

    #[inline]
    pub fn r(&self) -> f32 {
        self.r
    }

    #[inline]
    pub fn g(&self) -> f32 {
        self.g
    }

    #[inline]
    pub fn b(&self) -> f32 {
        self.b
    }

    #[inline]
    pub fn a(&self) -> f32 {
        self.a
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

    /// Converts a css color, or even #rrggbbaa to a Color
    pub fn from_css(css_color: &str) -> Self {
        let Ok(ccp_color) = ccpColor::from_html(css_color) else {
            log::error!("Failed to parse css color: {}", css_color);
            return Color::BLACK;
        };

        Self {
            r: ccp_color.r,
            g: ccp_color.g,
            b: ccp_color.b,
            a: ccp_color.a,
        }
    }
}

impl Default for Color {
    fn default() -> Self {
        Color::BLACK
    }
}