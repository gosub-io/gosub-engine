use gosub_shared::render_backend::Color as TColor;
use peniko::Color as PColor;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color {
    pub color: PColor,
}

impl Color {
    pub const fn rgba8(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self {
            color: PColor::rgba8(r, g, b, a)
        }
    }
}

impl TColor for Color {
    fn with_alpha(r: u8, g: u8, b: u8, a: u8) -> Self {
        Color::rgba8(r, g, b, a)
    }

    fn r(&self) -> u8 {
        self.color.r
    }

    fn g(&self) -> u8 {
        self.color.g
    }

    fn b(&self) -> u8 {
        self.color.b
    }

    fn a(&self) -> u8 {
        self.color.a
    }

    const WHITE: Self = Color::rgba8(255, 255, 255, 255);
    const BLACK: Self = Color::rgba8(0, 0, 0, 255);
    const RED: Self = Color::rgba8(255, 0, 0, 255);
    const GREEN: Self = Color::rgba8(0, 255, 0, 255);
    const BLUE: Self = Color::rgba8(0, 0, 255, 255);
    const YELLOW: Self = Color::rgba8(255, 255, 0, 255);
    const CYAN: Self = Color::rgba8(0, 255, 255, 255);
    const MAGENTA: Self = Color::rgba8(255, 0, 255, 255);
    const TRANSPARENT: Self = Color::rgba8(255, 255, 255, 0);
}
