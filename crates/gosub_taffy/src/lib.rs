use std::ops::{Deref, DerefMut};
use std::vec::IntoIter;
use taffy::{
    compute_block_layout, compute_cached_layout, compute_flexbox_layout, compute_grid_layout, compute_hidden_layout,
    compute_root_layout, AvailableSpace, Cache as TaffyCache, CacheTree, Display as TaffyDisplay,
    Layout as TaffyLayout, LayoutBlockContainer, LayoutFlexboxContainer, LayoutGridContainer, LayoutInput,
    LayoutOutput, LayoutPartialTree, NodeId as TaffyId, RunMode, SizingMode, Style, TraversePartialTree,
};

use gosub_interface::config::HasLayouter;
use gosub_interface::font::HasFontManager;
use gosub_interface::layout::{Layout as TLayout, LayoutCache, LayoutNode, LayoutTree, Layouter};
use gosub_shared::geo::{Point, Rect, Size, SizeU32};
use gosub_shared::types::Result;

use crate::compute::inline::compute_inline_layout;
use crate::style::get_style_from_node;
use crate::text::TextLayout;

mod compute;
pub mod style;
mod text;

/// Our layout implementation is based on Taffy properties.
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
        if size.width == f32::MIN && size.height == f32::MIN {
            None
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

/// Our implementation of the Taffy layouter.
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

#[derive(Default, Debug)]
#[allow(unused)]
pub struct Cache {
    taffy: TaffyCache,
    style: Style,
    display: Display,
}

impl Deref for Cache {
    type Target = TaffyCache;

    fn deref(&self) -> &Self::Target {
        &self.taffy
    }
}

impl DerefMut for Cache {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.taffy
    }
}

impl LayoutCache for Cache {
    fn invalidate(&mut self) {
        self.taffy.clear();
    }
}

impl<B: HasLayouter<Layouter = TaffyLayouter> + HasFontManager> Layouter<B> for TaffyLayouter {
    type Cache = Cache;
    type Layout = Layout;
    type TextLayout = TextLayout;

    const COLLAPSE_INLINE: bool = true;

    fn layout(
        &self,
        tree: &mut B::LayoutTree,
        root: <B::LayoutTree as LayoutTree<B>>::NodeId,
        space: SizeU32,
    ) -> Result<()> {
        let size = taffy::Size {
            width: AvailableSpace::Definite(space.width as f32),
            height: AvailableSpace::Definite(space.height as f32),
        };

        // We need to convert our tree into a LayoutDocument. This document can be used by Taffy to layout the tree
        // throughout the LayoutPartialTree trait that our LayoutDocument implements.
        let mut tree: LayoutDocument<B> = LayoutDocument(tree);

        // Precompute the styles for all nodes in the layout tree. This will convert all the CSS properties we need
        // for layouting into Taffy properties that are stored in a cache.
        Self::precompute_style(&mut tree, root);

        // Now let taffy compute the layout of the tree.
        compute_root_layout(&mut tree, TaffyId::from(root.into()), size);

        Ok(())
    }
}

impl TaffyLayouter {
    fn precompute_style<C: HasLayouter<Layouter = TaffyLayouter>>(
        tree: &mut LayoutDocument<C>,
        root: <C::LayoutTree as LayoutTree<C>>::NodeId,
    ) {
        // Convert our CSS properties into Taffy properties and store them in a cache.
        tree.update_style(root);

        let Some(children) = tree.0.children(root) else {
            return;
        };

        // Recursively precompute the style for all children of the current node.
        for child in children {
            Self::precompute_style(tree, <C::LayoutTree as LayoutTree<C>>::NodeId::from(child.into()));
        }
    }
}

#[repr(transparent)]
pub struct LayoutDocument<'a, C: HasLayouter>(&'a mut C::LayoutTree);

impl<C: HasLayouter<Layouter = TaffyLayouter>> TraversePartialTree for LayoutDocument<'_, C> {
    type ChildIter<'a>
        = IntoIter<TaffyId>
    where
        Self: 'a;

    fn child_ids(&self, parent: TaffyId) -> Self::ChildIter<'_> {
        let parent = <C::LayoutTree as LayoutTree<C>>::NodeId::from(parent.into());

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
        let parent = <C::LayoutTree as LayoutTree<C>>::NodeId::from(parent.into());

        self.0.child_count(parent)
    }

    fn get_child_id(&self, parent: TaffyId, index: usize) -> TaffyId {
        let parent = <C::LayoutTree as LayoutTree<C>>::NodeId::from(parent.into());

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

impl<C: HasLayouter<Layouter = TaffyLayouter>> LayoutDocument<'_, C> {
    /// Get the CSS properties for the given node, and store it inside the cache
    fn update_style(&mut self, node_id: <C::LayoutTree as LayoutTree<C>>::NodeId) {
        let Some(node) = self.0.get_node_mut(node_id) else {
            return;
        };

        let (style, display) = get_style_from_node(node);

        if let Some(cache) = self.0.get_cache_mut(node_id) {
            cache.style = style;
            cache.display = display;
        }
    }

    /// Get the taffy style properties for a given node. If the style is dirty, we will update the style first.
    fn get_taffy_style(&mut self, node_id: <C::LayoutTree as LayoutTree<C>>::NodeId) -> &Style {
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

    /// Force the taffy style from the cache. Do not care about dirty styles
    fn get_taffy_style_no_update(&self, node_id: <C::LayoutTree as LayoutTree<C>>::NodeId) -> &Style {
        if let Some(cache) = self.0.get_cache(node_id) {
            return &cache.style;
        }
        panic!(
            "Cache not found, why again does taffy don't use optionals? (node: {})",
            node_id.into()
        );
    }
}

impl<C: HasLayouter<Layouter = TaffyLayouter>> CacheTree for LayoutDocument<'_, C> {
    fn cache_get(
        &self,
        node_id: TaffyId,
        known_dimensions: taffy::Size<Option<f32>>,
        available_space: taffy::Size<AvailableSpace>,
        run_mode: RunMode,
    ) -> Option<LayoutOutput> {
        let node_id = <C::LayoutTree as LayoutTree<C>>::NodeId::from(node_id.into());
        let cache = &self
            .0
            .get_cache(node_id)
            .expect("Cache not found, why again does taffy don't use optionals?")
            .taffy;

        cache.get(known_dimensions, available_space, run_mode)
    }

    fn cache_store(
        &mut self,
        node_id: TaffyId,
        known_dimensions: taffy::Size<Option<f32>>,
        available_space: taffy::Size<AvailableSpace>,
        run_mode: RunMode,
        layout_output: LayoutOutput,
    ) {
        let node_id = <C::LayoutTree as LayoutTree<C>>::NodeId::from(node_id.into());
        let cache = &mut self
            .0
            .get_cache_mut(node_id)
            .expect("Cache not found, why again does taffy don't use optionals?")
            .taffy;

        cache.store(known_dimensions, available_space, run_mode, layout_output);
    }

    fn cache_clear(&mut self, node_id: TaffyId) {
        let node_id = <C::LayoutTree as LayoutTree<C>>::NodeId::from(node_id.into());
        let cache = &mut self
            .0
            .get_cache_mut(node_id)
            .expect("Cache not found, why again does taffy don't use optionals?")
            .taffy;

        cache.clear();
    }
}

/// Implementation of taffy's LayoutPartialTree
impl<C: HasLayouter<Layouter = TaffyLayouter>> LayoutPartialTree for LayoutDocument<'_, C> {
    type CoreContainerStyle<'a>
        = &'a Style
    where
        Self: 'a;
    fn get_core_container_style(&self, node_id: TaffyId) -> Self::CoreContainerStyle<'_> {
        self.get_taffy_style_no_update(<C::LayoutTree as LayoutTree<C>>::NodeId::from(node_id.into()))
    }

    fn set_unrounded_layout(&mut self, node_id: TaffyId, layout: &TaffyLayout) {
        let layout = Layout(*layout);

        let node_id = <C::LayoutTree as LayoutTree<C>>::NodeId::from(node_id.into());

        self.0.set_layout(node_id, layout);
    }

    fn compute_child_layout(&mut self, node_id: TaffyId, mut inputs: LayoutInput) -> LayoutOutput {
        inputs.sizing_mode = SizingMode::InherentSize;

        compute_cached_layout(self, node_id, inputs, |tree, node_id_taffy, inputs| {
            let node_id = <C::LayoutTree as LayoutTree<C>>::NodeId::from(node_id_taffy.into());

            if let Some(node) = tree.0.get_node_mut(node_id) {
                // If we are an inline parent, we should compute the inline layout
                if node.is_anon_inline_parent() {
                    println!("Node: {:?} is inline parent", node_id);
                    // Any text nodes are always inline, so they are handled in this function
                    return compute_inline_layout(tree, node_id, inputs);
                }
            }

            // let has_children = tree.0.child_count(node_id) > 0; //TODO: this isn't optimal, since we are now requesting the same node twice (up in get_cache and here)
            let style = tree.get_taffy_style(node_id);

            // @TODO: somehow we should implement table layout here as well. This could be doable with a Grid layout aparently.

            match style.display {
                TaffyDisplay::None => compute_hidden_layout(tree, node_id_taffy),
                TaffyDisplay::Block => compute_block_layout(tree, node_id_taffy, inputs),
                TaffyDisplay::Flex => compute_flexbox_layout(tree, node_id_taffy, inputs),
                TaffyDisplay::Grid => compute_grid_layout(tree, node_id_taffy, inputs),
            }
        })
    }
}

/// Implementation of taffy's LayoutBLockContainer
impl<C: HasLayouter<Layouter = TaffyLayouter>> LayoutBlockContainer for LayoutDocument<'_, C> {
    type BlockContainerStyle<'a>
        = &'a Style
    where
        Self: 'a;
    type BlockItemStyle<'a>
        = &'a Style
    where
        Self: 'a;

    fn get_block_container_style(&self, node_id: TaffyId) -> Self::BlockContainerStyle<'_> {
        self.get_taffy_style_no_update(<C::LayoutTree as LayoutTree<C>>::NodeId::from(node_id.into()))
    }

    fn get_block_child_style(&self, child_node_id: TaffyId) -> Self::BlockItemStyle<'_> {
        self.get_taffy_style_no_update(<C::LayoutTree as LayoutTree<C>>::NodeId::from(child_node_id.into()))
    }
}

/// Implementation of taffy's LayoutFlexboxContainer
impl<C: HasLayouter<Layouter = TaffyLayouter>> LayoutFlexboxContainer for LayoutDocument<'_, C> {
    type FlexboxContainerStyle<'a>
        = &'a Style
    where
        Self: 'a;
    type FlexboxItemStyle<'a>
        = &'a Style
    where
        Self: 'a;

    fn get_flexbox_container_style(&self, node_id: TaffyId) -> Self::FlexboxContainerStyle<'_> {
        self.get_taffy_style_no_update(<C::LayoutTree as LayoutTree<C>>::NodeId::from(node_id.into()))
    }

    fn get_flexbox_child_style(&self, child_node_id: TaffyId) -> Self::FlexboxItemStyle<'_> {
        self.get_taffy_style_no_update(<C::LayoutTree as LayoutTree<C>>::NodeId::from(child_node_id.into()))
    }
}

/// Implementation of taffy's LayoutGridContainer
impl<C: HasLayouter<Layouter = TaffyLayouter>> LayoutGridContainer for LayoutDocument<'_, C> {
    type GridContainerStyle<'a>
        = &'a Style
    where
        Self: 'a;
    type GridItemStyle<'a>
        = &'a Style
    where
        Self: 'a;

    fn get_grid_container_style(&self, node_id: TaffyId) -> Self::GridContainerStyle<'_> {
        self.get_taffy_style_no_update(<C::LayoutTree as LayoutTree<C>>::NodeId::from(node_id.into()))
    }

    fn get_grid_child_style(&self, child_node_id: TaffyId) -> Self::GridItemStyle<'_> {
        self.get_taffy_style_no_update(<C::LayoutTree as LayoutTree<C>>::NodeId::from(child_node_id.into()))
    }
}
