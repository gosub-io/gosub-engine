use crate::common::media::MediaId;
use crate::painter::commands::color::Color;
use crate::painter::commands::gradient::{Gradient, Tiling};

#[derive(Clone, Debug)]
pub enum Brush {
    /// Paint with fixed solid color
    Solid(Color),
    /// Paint with an image. `Some(tiling)` repeats the image as a CSS `background-image` tile
    /// (honouring `background-repeat`/`-size`/`-position`); `None` scales the image to fill the
    /// destination rect (a foreground `<img>` or a `background-size: cover/contain` background).
    Image(MediaId, Option<Tiling>),
    /// Paint with a CSS gradient (e.g. `linear-gradient(...)`).
    Gradient(Gradient),
}

impl Brush {
    pub fn solid(color: Color) -> Self {
        Brush::Solid(color)
    }

    /// A non-tiled image brush that scales the image to fill its destination rect.
    pub fn image(media_id: MediaId) -> Self {
        Brush::Image(media_id, None)
    }

    /// An image brush that tiles when `tiling` is `Some`, otherwise scales to fill.
    pub fn image_tiled(media_id: MediaId, tiling: Option<Tiling>) -> Self {
        Brush::Image(media_id, tiling)
    }

    pub fn gradient(gradient: Gradient) -> Self {
        Brush::Gradient(gradient)
    }
}
