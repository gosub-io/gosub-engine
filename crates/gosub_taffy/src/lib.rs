use std::vec::IntoIter;

use taffy::{
    compute_block_layout, compute_cached_layout, compute_flexbox_layout, compute_grid_layout, compute_hidden_layout,
    compute_root_layout, AvailableSpace, Cache as TaffyCache, Display as TaffyDisplay, Layout as TaffyLayout,
    LayoutInput, LayoutOutput, LayoutPartialTree, NodeId as TaffyId, Style, TraversePartialTree,
};

use gosub_render_backend::geo::{Point, Rect, Size, SizeU32};
use gosub_render_backend::layout::{Layout as TLayout, LayoutTree, Layouter, Node};
use gosub_shared::types::Result;

use crate::compute::inline::compute_inline_layout;
use crate::style::get_style_from_node;
use crate::text::TextLayout;

mod compute;
mod conversions;
pub mod style;
mod text;

#[repr(transparent)]
#[derive(Default, Debug)]
pub struct Layout(TaffyLayout);

impl TLayout for Layout {
    fn rel_pos(&self) -> Point {
        let pos = self.0.location;

        Point::new(pos.x, pos.y)
    }

    fn z_index(&self) -> u32 {
        self.0.order
    }

    fn size(&self) -> Size {
        let size = self.0.size;
        Size::new(size.width, size.height)
    }

    fn size_or(&self) -> Option<Size> {
        let size = self.0.size;
        if size.width == 0.0 && size.height == 0.0 {
            None //Temporary hack to indicate that the size is not set
        } else {
            Some(Size::new(size.width, size.height))
        }
    }

    fn set_size(&mut self, size: SizeU32) {
        self.0.size = taffy::Size {
            width: size.width as f32,
            height: size.height as f32,
        };
    }

    fn set_content(&mut self, size: SizeU32) {
        self.0.content_size = taffy::Size {
            width: size.width as f32,
            height: size.height as f32,
        };
    }

    fn content(&self) -> Size {
        let content = self.0.content_size;
        Size::new(content.width, content.height)
    }

    fn scrollbar(&self) -> Size {
        let scroll = self.0.scrollbar_size;
        Size::new(scroll.width, scroll.height)
    }

    fn border(&self) -> Rect {
        let border = self.0.border;
        Rect::new(border.top, border.right, border.bottom, border.left)
    }

    fn padding(&self) -> Rect {
        let padding = self.0.padding;
        Rect::new(padding.top, padding.right, padding.bottom, padding.left)
    }

    fn margin(&self) -> Rect {
        Rect::new(0.0, 0.0, 0.0, 0.0) // Taffy doesn't have margin in its layout
    }
}

#[derive(Clone, Copy, Debug)]
pub struct TaffyLayouter;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[allow(unused)]
pub enum Display {
    Inline,
    InlineBlock,
    Table,
    #[default]
    Taffy,
}

#[derive(Default)]
#[allow(unused)]
pub struct Cache {
    taffy: TaffyCache,
    style: Style,
    display: Display,
}

impl Layouter for TaffyLayouter {
    type Cache = Cache;
    type Layout = Layout;
    type TextLayout = TextLayout;

    const COLLAPSE_INLINE: bool = true;

    fn layout<LT: LayoutTree<Self>>(&self, tree: &mut LT, root: LT::NodeId, space: SizeU32) -> Result<()> {

        println!("Layouting tree with root {:?}", space);

        let size = taffy::Size {
            width: AvailableSpace::Definite(space.width as f32),
            height: AvailableSpace::Definite(space.height as f32),
        };

        let mut tree = LayoutDocument(tree);
        Self::precompute_style(&mut tree, root);
        compute_root_layout(&mut tree, TaffyId::from(root.into()), size);

        Ok(())
    }
}

impl TaffyLayouter {
    fn precompute_style<LT: LayoutTree<Self>>(tree: &mut LayoutDocument<LT>, root: LT::NodeId) {
        tree.update_style(root);

        let Some(children) = tree.0.children(root) else {
            return;
        };

        for child in children {
            Self::precompute_style(tree, LT::NodeId::from(child.into()));
        }
    }
}

#[repr(transparent)]
pub struct LayoutDocument<'a, LT: LayoutTree<TaffyLayouter>>(&'a mut LT);

impl<LT: LayoutTree<TaffyLayouter>> TraversePartialTree for LayoutDocument<'_, LT> {
    type ChildIter<'a> = IntoIter<TaffyId>
    where
        Self: 'a;

    fn child_ids(&self, parent: TaffyId) -> Self::ChildIter<'_> {
        let parent = LT::NodeId::from(parent.into());

        if let Some(children) = self.0.children(parent) {
            children
                .iter()
                .filter(|id| self.0.contains(id)) //FIXME: This is a hack, we should not have to filter out non-existing nodes
                .map(|id| TaffyId::from(Into::into(*id)))
                .collect::<Vec<_>>()
                .into_iter()
        } else {
            Vec::new().into_iter()
        }
    }

    fn child_count(&self, parent: TaffyId) -> usize {
        let parent = LT::NodeId::from(parent.into());

        self.0.child_count(parent)
    }

    fn get_child_id(&self, parent: TaffyId, index: usize) -> TaffyId {
        let parent = LT::NodeId::from(parent.into());

        if let Some(node) = self.0.children(parent) {
            TaffyId::from(
                node.into_iter()
                    .filter(|id| self.0.contains(id)) //FIXME: This is a hack, we should not have to filter out non-existing nodes
                    .nth(index)
                    .map(Into::into)
                    .unwrap_or_default(),
            )
        } else {
            TaffyId::from(0u64) //TODO: this maybe shouldn't be 0
        }
    }
}

impl<LT: LayoutTree<TaffyLayouter>> LayoutDocument<'_, LT> {
    fn update_style(&mut self, node_id: LT::NodeId) {
        let Some(node) = self.0.get_node(node_id) else {
            return;
        };

        let (style, display) = get_style_from_node(node);

        if let Some(cache) = self.0.get_cache_mut(node_id) {
            cache.style = style;
            cache.display = display;
        }
    }

    fn get_taffy_style(&mut self, node_id: LT::NodeId) -> &Style {
        let dirty_style = self.0.style_dirty(node_id);

        if dirty_style {
            self.update_style(node_id);
        }

        let cache = self
            .0
            .get_cache(node_id)
            .expect("Cache not found, why again does taffy don't use optionals?");

        &cache.style
    }

    fn get_taffy_style_no_update(&self, node_id: LT::NodeId) -> &Style {
        if let Some(cache) = self.0.get_cache(node_id) {
            return &cache.style;
        }
        panic!(
            "Cache not found, why again does taffy don't use optionals? (node: {})",
            node_id.into()
        );
    }
}

impl<LT: LayoutTree<TaffyLayouter>> LayoutPartialTree for LayoutDocument<'_, LT> {
    fn get_style(&self, node_id: TaffyId) -> &Style {
        self.get_taffy_style_no_update(LT::NodeId::from(node_id.into()))
    }

    fn set_unrounded_layout(&mut self, node_id: TaffyId, layout: &TaffyLayout) {
        let layout = Layout(*layout);

        let node_id = LT::NodeId::from(node_id.into());

        self.0.set_layout(node_id, layout);
    }

    fn get_cache_mut(&mut self, node_id: TaffyId) -> &mut TaffyCache {
        let node_id = LT::NodeId::from(node_id.into());
        &mut self
            .0
            .get_cache_mut(node_id)
            .expect("Cache not found, why again does taffy don't use optionals?")
            .taffy
    }

    fn compute_child_layout(&mut self, node_id: TaffyId, inputs: LayoutInput) -> LayoutOutput {
        compute_cached_layout(self, node_id, inputs, |tree, node_id_taffy, inputs| {
            let node_id = LT::NodeId::from(node_id_taffy.into());

            if let Some(node) = tree.0.get_node(node_id) {
                if node.is_anon_inline_parent() {
                    return compute_inline_layout(tree, node_id, inputs);
                }
            }

            // let has_children = tree.0.child_count(node_id) > 0; //TODO: this isn't optimal, since we are now requesting the same node twice (up in get_cache and here)
            let style = tree.get_taffy_style(node_id);

            match style.display {
                TaffyDisplay::None => compute_hidden_layout(tree, node_id_taffy),
                TaffyDisplay::Block => compute_block_layout(tree, node_id_taffy, inputs),
                TaffyDisplay::Flex => compute_flexbox_layout(tree, node_id_taffy, inputs),
                TaffyDisplay::Grid => compute_grid_layout(tree, node_id_taffy, inputs),
            }
        })
    }
}
