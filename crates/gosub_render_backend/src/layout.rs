use gosub_shared::traits::css3::CssProperty;
use std::fmt::Debug;

use gosub_shared::types::Result;
use gosub_typeface::font::{Font, Glyph};

use crate::geo::{Point, Rect, Size, SizeU32};

pub trait LayoutTree<L: Layouter>: Sized + 'static{
    type NodeId: Copy + Clone + From<u64> + Into<u64>;
    type Node: Node + HasTextLayout<L>;

    fn children(&self, id: Self::NodeId) -> Option<Vec<Self::NodeId>>;
    fn contains(&self, id: &Self::NodeId) -> bool;
    fn child_count(&self, id: Self::NodeId) -> usize;
    fn parent_id(&self, id: Self::NodeId) -> Option<Self::NodeId>;
    fn get_cache(&self, id: Self::NodeId) -> Option<&L::Cache>;
    fn get_layout(&self, id: Self::NodeId) -> Option<&L::Layout>;
    fn get_cache_mut(&mut self, id: Self::NodeId) -> Option<&mut L::Cache>;
    fn get_layout_mut(&mut self, id: Self::NodeId) -> Option<&mut L::Layout>;
    fn set_cache(&mut self, id: Self::NodeId, cache: L::Cache);
    fn set_layout(&mut self, id: Self::NodeId, layout: L::Layout);

    fn style_dirty(&self, id: Self::NodeId) -> bool;

    fn clean_style(&mut self, id: Self::NodeId);

    fn get_node(&mut self, id: Self::NodeId) -> Option<&mut Self::Node>;
}

pub trait Layouter: Sized + Clone + Send + 'static {
    type Cache: Default + Send;
    type Layout: Layout + Send;

    type TextLayout: TextLayout + Send;

    const COLLAPSE_INLINE: bool;

    fn layout<LT: LayoutTree<Self>>(&self, tree: &mut LT, root: LT::NodeId, space: SizeU32) -> Result<()>;
}

pub trait Layout: Default {
    /// Returns the relative upper left pos of the content box
    fn rel_pos(&self) -> Point;

    /// Returns the z-index of the element
    fn z_index(&self) -> u32;

    /// Size of the scroll box (content box without overflow), including scrollbars (if any)
    fn size(&self) -> Size;

    fn size_or(&self) -> Option<Size>;

    fn set_size_and_content(&mut self, size: SizeU32) {
        self.set_size(size);
        self.set_content(size);
    }

    fn set_size(&mut self, size: SizeU32);

    fn set_content(&mut self, size: SizeU32);

    /// Size of the content box (content without scrollbars, but with overflow)
    fn content(&self) -> Size;
    fn content_box(&self) -> Rect {
        let pos = self.rel_pos();
        let size = self.size();
        Rect::from_components(pos, size)
    }

    /// Additional space taken up by the scrollbar
    fn scrollbar(&self) -> Size;
    fn scrollbar_box(&self) -> Rect {
        let pos = self.rel_pos();
        let content = self.content();
        let size = self.scrollbar();
        Rect::new(pos.x, pos.y, content.width + size.width, content.height + size.height)
    }

    fn border(&self) -> Rect;
    fn border_box(&self) -> Rect {
        let pos = self.rel_pos();
        let size = self.size();
        let border = self.border();

        Rect::new(
            pos.x - border.x1,
            pos.y - border.y1,
            size.width + border.x2,
            size.height + border.y2,
        )
    }

    fn padding(&self) -> Rect;
    fn padding_box(&self) -> Rect {
        let pos = self.rel_pos();
        let border = self.border();
        Rect::from_components(pos, border.size())
    }

    fn margin(&self) -> Rect;
    fn margin_box(&self) -> Rect {
        let border = self.border_box();
        let margin = self.margin();

        Rect::new(
            border.x1 - margin.x1,
            border.y1 - margin.y1,
            border.x2 + margin.x2,
            border.y2 + margin.y2,
        )
    }
}

pub trait Node {
    type Property: CssProperty;

    fn get_property(&self, name: &str) -> Option<&Self::Property>;
    fn text_data(&self) -> Option<&str>;

    fn text_size(&self) -> Option<Size>;

    /// This can only return true if the `Layout::COLLAPSE_INLINE` is set true for the layouter
    ///
    fn is_anon_inline_parent(&self) -> bool;
}

pub trait HasTextLayout<L: Layouter> {
    fn set_text_layout(&mut self, layout: L::TextLayout);
}

pub trait TextLayout {
    type Font: Font;
    fn dbg_layout(&self) -> String;

    fn size(&self) -> Size;

    fn glyphs(&self) -> &[Glyph];

    fn font(&self) -> &Self::Font;

    fn font_size(&self) -> f32;

    fn coords(&self) -> &[i16];

    fn decorations(&self) -> &Decoration;
}

#[derive(Debug, Clone, Default)]
pub struct Decoration {
    pub underline: bool,
    pub overline: bool,
    pub line_through: bool,

    pub color: (f32, f32, f32, f32),
    pub style: DecorationStyle,
    pub width: f32,

    pub underline_offset: f32,

    pub x_offset: f32,
}

#[derive(Debug, Clone, Default)]
pub enum DecorationStyle {
    #[default]
    Solid,
    Double,
    Dotted,
    Dashed,
    Wavy,
}
