use crate::common::media::MediaId;
use crate::painter::commands::color::Color;
use crate::painter::commands::gradient::{Gradient, Tiling};

#[derive(Clone, Debug)]
pub enum Brush {
    Solid(Color),
    /// `Some(tiling)` repeats per CSS `background-repeat`/`-size`/`-position`; `None` scales to
    /// fill the destination rect (foreground `<img>`, or `background-size: cover/contain`).
    Image(MediaId, Option<Tiling>),
    Gradient(Gradient),
}

impl Brush {
    pub fn solid(color: Color) -> Self {
        Brush::Solid(color)
    }

    /// Non-tiled: scales the image to fill its destination rect.
    pub fn image(media_id: MediaId) -> Self {
        Brush::Image(media_id, None)
    }

    pub fn image_tiled(media_id: MediaId, tiling: Option<Tiling>) -> Self {
        Brush::Image(media_id, tiling)
    }

    pub fn gradient(gradient: Gradient) -> Self {
        Brush::Gradient(gradient)
    }
}
