use gosub_render_backend::Color as TColor;
use vello::peniko::Color as VelloColor;

pub struct Color(pub(crate) VelloColor);

impl From<VelloColor> for Color {
    fn from(color: VelloColor) -> Self {
        Color(color)
    }
}

impl Color {
    pub const fn rgba8(r: u8, g: u8, b: u8, a: u8) -> Self {
        Color(VelloColor::rgba8(r, g, b, a))
    }
}

impl TColor for Color {
    fn with_alpha(r: u8, g: u8, b: u8, a: u8) -> Self {
        VelloColor::rgba8(r, g, b, a).into()
    }

    fn r(&self) -> u8 {
        self.0.r
    }

    fn g(&self) -> u8 {
        self.0.g
    }

    fn b(&self) -> u8 {
        self.0.b
    }

    fn a(&self) -> u8 {
        self.0.a
    }

    const WHITE: Self = Color(VelloColor::WHITE);
    const BLACK: Self = Color(VelloColor::BLACK);
    const RED: Self = Color(VelloColor::RED);
    const GREEN: Self = Color(VelloColor::GREEN);
    const BLUE: Self = Color(VelloColor::BLUE);
    const YELLOW: Self = Color(VelloColor::YELLOW);
    const CYAN: Self = Color(VelloColor::CYAN);
    const MAGENTA: Self = Color(VelloColor::MAGENTA);
    const TRANSPARENT: Self = Color(VelloColor::TRANSPARENT);
}
