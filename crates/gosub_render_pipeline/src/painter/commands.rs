use crate::common::media::MediaId;
use crate::painter::commands::rectangle::Rectangle;
use crate::painter::commands::text::Text;

pub mod color;
pub mod text;
pub mod image;
pub mod border;
pub mod rectangle;
pub mod brush;

/// Generic that defines a top, right, bottom, and left value.
#[derive(Clone, Debug)]
pub struct Trbl<T> {
    top: T,
    right: T,
    bottom: T,
    left: T,
}

#[derive(Clone, Debug)]
pub struct PaintSvg {
    pub rect: Rectangle,
    pub media_id: MediaId,
}

#[derive(Clone, Debug)]
pub enum PaintCommand {
    Text(Text),
    Rectangle(Rectangle),
    Svg(PaintSvg),
}

impl PaintCommand {
    pub fn text(text: Text) -> Self {
        PaintCommand::Text(text)
    }

    pub fn svg(media_id: MediaId, rect: Rectangle) -> Self {
        PaintCommand::Svg(PaintSvg{
            rect,
            media_id,
        })
    }

    pub fn rectangle(rectangle: Rectangle) -> Self {
        PaintCommand::Rectangle(rectangle)
    }
}