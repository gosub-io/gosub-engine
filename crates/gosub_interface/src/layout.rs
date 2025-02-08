use crate::config::HasLayouter;
use crate::font::{FontBlob, HasFontManager};
use gosub_shared::font::Glyph;
use gosub_shared::geo::{Point, Rect, Size, SizeU32};
use gosub_shared::types::Result;
use std::fmt::Debug;

#[derive(Clone, Debug)]
pub struct FontData {
    data: Vec<u8>,
    index: u32,
}

impl FontData {
    pub fn new(data: &[u8], index: u32) -> FontData {
        Self {
            data: data.to_vec(),
            index,
        }
    }

    pub fn to_bytes(&self) -> &[u8] {
        &self.data
    }

    pub fn index(&self) -> u32 {
        self.index
    }
}

/// LayoutTree is a combined structure of a RenderTree and a LayoutTree. The RenderTree part contains all the
/// nodes that can be rendered or have any effect of visual layout. The layout part holds all the information
/// about how the nodes are laid out on the screen.
pub trait LayoutTree<C: HasLayouter<LayoutTree = Self>>: Sized + Debug + 'static {
    type NodeId: Debug + Copy + Clone + From<u64> + Into<u64> + PartialEq;
    type Node: LayoutNode<C>;

    /// Returns all NodeIds of the children of the given NodeId
    fn children(&self, id: Self::NodeId) -> Option<Vec<Self::NodeId>>;
    /// Returns true when the given NodeId is a child of the root node
    fn contains(&self, id: &Self::NodeId) -> bool;
    /// Returns the count of the children
    fn child_count(&self, id: Self::NodeId) -> usize;
    /// Returns the parent of the given NodeId, or None when the node is a root node
    fn parent_id(&self, id: Self::NodeId) -> Option<Self::NodeId>;

    /// Returns the Layout cache which holds all the style and display information of a node
    fn get_cache(&self, id: Self::NodeId) -> Option<&<C::Layouter as Layouter<C>>::Cache>;
    /// Returns the layout data of the node.
    fn get_layout(&self, id: Self::NodeId) -> Option<&<C::Layouter as Layouter<C>>::Layout>;

    fn get_cache_mut(&mut self, id: Self::NodeId) -> Option<&mut <C::Layouter as Layouter<C>>::Cache>;
    fn get_layout_mut(&mut self, id: Self::NodeId) -> Option<&mut <C::Layouter as Layouter<C>>::Layout>;
    fn set_cache(&mut self, id: Self::NodeId, cache: <C::Layouter as Layouter<C>>::Cache);
    fn set_layout(&mut self, id: Self::NodeId, layout: <C::Layouter as Layouter<C>>::Layout);

    fn style_dirty(&self, id: Self::NodeId) -> bool;
    fn clean_style(&mut self, id: Self::NodeId);

    /// Get node functionality
    fn get_node_mut(&mut self, id: Self::NodeId) -> Option<&mut Self::Node>;
    fn get_node(&self, id: Self::NodeId) -> Option<&Self::Node>;

    /// Returns the root node of the tree
    fn root(&self) -> Self::NodeId;
}

/// Main layout trait that will convert a RenderTree into a LayoutTree (or in our case, it will
/// update the LayoutTree with new layout information)
pub trait Layouter<C: HasLayouter + HasFontManager>: Sized + Clone + Send + 'static {
    type Cache: LayoutCache;
    type Layout: Layout + Send;

    type TextLayout: TextLayout + Send + Debug;

    const COLLAPSE_INLINE: bool;

    fn layout(
        &self,
        // Rendertree, probably not filled with layout information. This rendertree will be updated by this function.
        tree: &mut C::LayoutTree,
        // The root node of the tree. This is not really needed as the LayoutTree also contains this information
        root: <C::LayoutTree as LayoutTree<C>>::NodeId,
        // Dimensions of the viewport that we layout in
        space: SizeU32,
    ) -> Result<()>;
}

/// Cache that holds all the style and display information of a node
pub trait LayoutCache: Default + Send + Debug {
    fn invalidate(&mut self);
}

/// Trait that defines all layout information of a node. Currently residing in the same tree that also
/// holds the RenderTree. This is not ideal, but it is a start.
pub trait Layout: Default + Debug {
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

pub trait LayoutNode<C: HasLayouter>: HasTextLayout<C> {
    fn get_property(&self, name: &str) -> Option<&C::CssProperty>;
    fn text_data(&self) -> Option<&str>;
    fn text_size(&self) -> Option<Size>;
    /// This can only return true if the `Layout::COLLAPSE_INLINE` is set true for the layouter
    fn is_anon_inline_parent(&self) -> bool;
}

pub trait HasTextLayout<C: HasLayouter> {
    fn clear_text_layout(&mut self);
    fn add_text_layout(&mut self, layout: <C::Layouter as Layouter<C>>::TextLayout);
    fn get_text_layouts(&self) -> Option<&[<C::Layouter as Layouter<C>>::TextLayout]>;
    fn get_text_layouts_mut(&mut self) -> Option<&mut Vec<<C::Layouter as Layouter<C>>::TextLayout>>;
}

/// Text layout that keeps all information on how a part of text is laid out
pub trait TextLayout {
    /// Returns a list of glyphs for the text
    fn glyphs(&self) -> &[Glyph];
    /// Font data
    fn font_data(&self) -> &FontBlob;
    // Size of the font in pixels
    fn font_size(&self) -> f32;
    /// Additional font decorations
    fn decorations(&self) -> &Decoration;
    // Offset?
    fn offset(&self) -> Point;
    /// Coordinates of the font
    fn coords(&self) -> &[i16];
    /// Size of the text
    fn size(&self) -> Size;
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
