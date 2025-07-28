use gosub_interface::render_backend::Color as TColor;
use vello::peniko::Color as VelloColor;

pub struct Color(pub(crate) VelloColor);

impl From<VelloColor> for Color {
    fn from(color: VelloColor) -> Self {
        Color(color)
    }
}

impl Color {
    #[must_use] 
    pub const fn rgba8(r: u8, g: u8, b: u8, a: u8) -> Self {
        Color(VelloColor::from_rgba8(r, g, b, a))
    }
}

impl TColor for Color {
    fn with_alpha(r: u8, g: u8, b: u8, a: u8) -> Self {
        VelloColor::from_rgba8(r, g, b, a).into()
    }

    fn r(&self) -> u8 {
        (self.0.components.as_slice()[0] * 255.0) as u8
    }

    fn g(&self) -> u8 {
        (self.0.components.as_slice()[1] * 255.0) as u8
    }

    fn b(&self) -> u8 {
        (self.0.components.as_slice()[2] * 255.0) as u8
    }

    fn a(&self) -> u8 {
        (self.0.components.as_slice()[3] * 255.0) as u8
    }

    const WHITE: Self = Color(vello::peniko::color::palette::css::WHITE);
    const BLACK: Self = Color(vello::peniko::color::palette::css::BLACK);
    const RED: Self = Color(vello::peniko::color::palette::css::RED);
    const GREEN: Self = Color(vello::peniko::color::palette::css::GREEN);
    const BLUE: Self = Color(vello::peniko::color::palette::css::BLUE);
    const YELLOW: Self = Color(vello::peniko::color::palette::css::YELLOW);
    const CYAN: Self = Color(vello::peniko::color::palette::css::CYAN);
    const MAGENTA: Self = Color(vello::peniko::color::palette::css::MAGENTA);
    const TRANSPARENT: Self = Color(vello::peniko::color::palette::css::TRANSPARENT);
}
