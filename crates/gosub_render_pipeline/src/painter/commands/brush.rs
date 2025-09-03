use crate::common::media::MediaId;
use crate::painter::commands::color::Color;
use crate::painter::commands::image::Image;

#[derive(Clone, Debug)]
pub enum Brush {
    /// Paint with fixed solid color
    Solid(Color),
    /// Paint with an image. This allows us to display images
    Image(MediaId),
    // Gradient(Gradient),
}

impl Brush {
    pub fn solid(color: Color) -> Self {
        Brush::Solid(color)
    }

    pub fn image(media_id: MediaId) -> Self {
        Brush::Image(media_id)
    }

    // pub fn gradient(gradient: Gradient) -> Self {
    //     Brush::Gradient(gradient)
    // }
}
