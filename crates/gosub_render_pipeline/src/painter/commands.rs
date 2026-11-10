use crate::common::geo::Rect;
use crate::common::media::MediaId;
use crate::painter::commands::rectangle::Rectangle;
use crate::painter::commands::text::Text;
use crate::render::backend::TileAnchor;

pub mod border;
pub mod brush;
pub mod color;
pub mod gradient;
pub mod image;
pub mod rectangle;
pub mod shadow;
pub mod text;

#[derive(Clone, Debug)]
pub struct Trbl<T> {
    pub top: T,
    pub right: T,
    pub bottom: T,
    pub left: T,
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
    /// Begin a compositing group for a promoted layer (`opacity < 1`, `position: fixed`/`sticky`):
    /// everything up to the matching [`PaintCommand::PopLayer`] is composited as a unit.
    /// Only the scene path (`Painter::paint_all`) emits these - the tile path applies opacity/anchor
    /// at composite time, so tile rasterizers can ignore both variants.
    PushLayer {
        opacity: f32,
        anchor: TileAnchor,
    },
    /// End the most recent [`PaintCommand::PushLayer`] group.
    PopLayer,
    /// Clip everything up to the matching [`PaintCommand::PopClip`] to this rect, in page
    /// coordinates. Both the tile and the scene path emit these.
    PushClip(Rect),
    /// End the most recent [`PaintCommand::PushClip`].
    PopClip,
}

impl PaintCommand {
    pub fn text(text: Text) -> Self {
        PaintCommand::Text(text)
    }

    pub fn svg(media_id: MediaId, rect: Rectangle) -> Self {
        PaintCommand::Svg(PaintSvg { rect, media_id })
    }

    pub fn rectangle(rectangle: Rectangle) -> Self {
        PaintCommand::Rectangle(rectangle)
    }
}
