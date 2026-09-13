//! What a change invalidates, and how much of the pipeline has to run again.
//!
//! The render pipeline has six stages (render tree, layout, layering, tiling, paint, raster)
//! and the cost of redoing them differs by orders of magnitude. Before this existed, a
//! `BrowsingContext` carried a handful of whole-document booleans, and everything except
//! hover funnelled into a full rebuild - so changing `:focus` on one element re-rasterized
//! the entire page.
//!
//! [`Damage`] records the *smallest* amount of work a frame needs: a level, plus the DOM
//! nodes whose computed styles went stale and the page-space rects that need repainting.
//! Several changes can land in one frame, so recording is monotonic - the level only ever
//! rises, and node and rect sets accumulate - which makes "hover moved" plus "an image
//! finished decoding" combine into the stronger of the two without the caller thinking about it.
//!
//! Scrolling is deliberately *not* a damage level: it invalidates nothing about the content,
//! only which cached tiles are on screen and how far the raster window reaches, which
//! `BrowsingContext`'s `scroll_dirty` / `raster_dirty` already track.

use gosub_render_pipeline::common::geo::Rect as PipelineRect;
use gosub_shared::node::NodeId;

/// How much of the pipeline a change invalidates.
///
/// Ordered weakest to strongest: a stronger level subsumes everything a weaker one implies,
/// which is what makes [`Damage::escalate`] a simple `max`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum DamageLevel {
    /// Nothing to do.
    #[default]
    None,
    /// Layout still holds, but some pixels are wrong. `:hover` and `:focus` styling.
    Paint,
    /// Boxes move, but every input to the layout tree is unchanged: same nodes, same styles,
    /// same intrinsic sizes. Only the geometry has to be recomputed, on the taffy tree that
    /// is already built. A viewport resize.
    ///
    /// Roughly half of layout time goes into *building* that tree (36ms of 74ms on a
    /// Wikipedia article), so this tier is worth having distinct from `Layout`.
    Geometry,
    /// The layout tree itself must be rebuilt, though computed styles are still valid. An
    /// image whose intrinsic size only became known once it decoded: that size is baked into
    /// the tree when it is generated, so recomputing geometry alone would not pick it up.
    Layout,
    /// Computed styles are stale and must be recomputed before layout can run. A media
    /// breakpoint flipped, or a rule set changed.
    Style,
    /// Start over: a different document, or a structural DOM change. Nothing is reusable.
    Rebuild,
}

impl DamageLevel {
    /// Whether this level requires computed styles to be recomputed.
    #[must_use]
    pub fn needs_restyle(self) -> bool {
        self >= DamageLevel::Style
    }

    /// Whether the layout tree has to be built again, as opposed to merely recomputed.
    #[must_use]
    pub fn needs_layout_tree(self) -> bool {
        self >= DamageLevel::Layout
    }

    /// Whether box geometry has to be recomputed at all.
    #[must_use]
    pub fn needs_geometry(self) -> bool {
        self >= DamageLevel::Geometry
    }
}

/// The accumulated damage for the next frame.
///
/// Cleared by [`Damage::take`] once the pipeline has acted on it.
#[derive(Debug, Clone, Default)]
pub struct Damage {
    level: DamageLevel,
    /// DOM nodes whose cached computed styles are stale.
    ///
    /// Empty at [`DamageLevel::Rebuild`], where everything is stale and listing nodes would
    /// be pointless. Kept small and deduplicated: these drive targeted cache invalidation.
    nodes: Vec<NodeId>,
    /// Page-space rects known to need repainting. Empty means "the consumer works it out",
    /// which for a `Layout`-or-worse level means the whole page anyway.
    rects: Vec<PipelineRect>,
}

impl Damage {
    #[must_use]
    pub fn none() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn level(&self) -> DamageLevel {
        self.level
    }

    #[must_use]
    pub fn is_none(&self) -> bool {
        self.level == DamageLevel::None
    }

    #[must_use]
    pub fn nodes(&self) -> &[NodeId] {
        &self.nodes
    }

    #[must_use]
    pub fn rects(&self) -> &[PipelineRect] {
        &self.rects
    }

    /// Raise the level to at least `level`, leaving it alone if it is already stronger.
    pub fn escalate(&mut self, level: DamageLevel) {
        self.level = self.level.max(level);
        if self.level == DamageLevel::Rebuild {
            // Everything is stale, so the per-node and per-rect detail is dead weight.
            self.nodes.clear();
            self.rects.clear();
        }
    }

    /// Record that `node`'s computed styles are stale, without moving the level.
    ///
    /// Callers pair this with an `escalate` describing how far the consequences reach: a
    /// `:hover` change is `Paint`, a class change that resizes a box is `Layout`.
    pub fn add_node(&mut self, node: NodeId) {
        if self.level == DamageLevel::Rebuild || self.nodes.contains(&node) {
            return;
        }
        self.nodes.push(node);
    }

    /// Record several stale nodes at once.
    pub fn add_nodes(&mut self, nodes: impl IntoIterator<Item = NodeId>) {
        for node in nodes {
            self.add_node(node);
        }
    }

    /// Record a page-space rect that needs repainting.
    pub fn add_rect(&mut self, rect: PipelineRect) {
        if self.level == DamageLevel::Rebuild {
            return;
        }
        self.rects.push(rect);
    }

    /// Styles on `node` went stale and the result is visible but does not move anything:
    /// `:hover`, `:focus`, a colour change.
    pub fn style_changed_paint_only(&mut self, node: NodeId) {
        self.escalate(DamageLevel::Paint);
        self.add_node(node);
    }

    /// Everything must be recomputed - a new document, or a structural DOM change.
    pub fn rebuild(&mut self) {
        self.escalate(DamageLevel::Rebuild);
    }

    /// The union of the recorded rects, or `None` when none were recorded.
    ///
    /// Consumers repaint the tiles this covers and carry the rest forward, so an empty result
    /// means "nothing localised to repaint", not "repaint everything" - the level says that.
    #[must_use]
    pub fn bounding_rect(&self) -> Option<PipelineRect> {
        self.rects.iter().copied().reduce(|a, b| {
            let x0 = a.x.min(b.x);
            let y0 = a.y.min(b.y);
            let x1 = (a.x + a.width).max(b.x + b.width);
            let y1 = (a.y + a.height).max(b.y + b.height);
            PipelineRect::new(x0, y0, x1 - x0, y1 - y0)
        })
    }

    /// Take the accumulated damage, leaving this reset to [`DamageLevel::None`].
    #[must_use]
    pub fn take(&mut self) -> Damage {
        std::mem::take(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: usize) -> NodeId {
        NodeId::from(id)
    }

    #[test]
    fn escalation_is_monotonic() {
        let mut damage = Damage::none();
        assert!(damage.is_none());

        damage.escalate(DamageLevel::Paint);
        assert_eq!(damage.level(), DamageLevel::Paint);

        // A stronger level wins.
        damage.escalate(DamageLevel::Layout);
        assert_eq!(damage.level(), DamageLevel::Layout);

        // A weaker one does not pull it back down - the frame still needs the layout work.
        damage.escalate(DamageLevel::Paint);
        assert_eq!(damage.level(), DamageLevel::Layout);
    }

    #[test]
    fn levels_report_what_they_need() {
        assert!(!DamageLevel::Paint.needs_geometry());
        assert!(!DamageLevel::Paint.needs_layout_tree());
        assert!(!DamageLevel::Paint.needs_restyle());

        // A resize recomputes geometry on the tree it already has.
        assert!(DamageLevel::Geometry.needs_geometry());
        assert!(!DamageLevel::Geometry.needs_layout_tree());
        assert!(!DamageLevel::Geometry.needs_restyle());

        assert!(DamageLevel::Layout.needs_layout_tree());
        assert!(!DamageLevel::Layout.needs_restyle());
        assert!(DamageLevel::Style.needs_layout_tree());
        assert!(DamageLevel::Style.needs_restyle());
        assert!(DamageLevel::Rebuild.needs_restyle());
    }

    #[test]
    fn nodes_accumulate_without_duplicates() {
        let mut damage = Damage::none();
        damage.style_changed_paint_only(node(1));
        damage.style_changed_paint_only(node(2));
        damage.style_changed_paint_only(node(1));

        assert_eq!(damage.level(), DamageLevel::Paint);
        assert_eq!(damage.nodes(), &[node(1), node(2)]);
    }

    /// Two independent changes in one frame combine into the stronger of the two, and the
    /// weaker one's node detail survives - it still needs its style recomputed.
    #[test]
    fn independent_changes_combine() {
        let mut damage = Damage::none();
        damage.style_changed_paint_only(node(1));
        damage.escalate(DamageLevel::Layout);
        damage.add_node(node(2));

        assert_eq!(damage.level(), DamageLevel::Layout);
        assert_eq!(damage.nodes(), &[node(1), node(2)]);
    }

    /// A rebuild makes per-node detail meaningless, so it is dropped - and stays dropped.
    #[test]
    fn rebuild_discards_detail() {
        let mut damage = Damage::none();
        damage.style_changed_paint_only(node(1));
        damage.add_rect(PipelineRect::new(0.0, 0.0, 10.0, 10.0));
        damage.rebuild();

        assert_eq!(damage.level(), DamageLevel::Rebuild);
        assert!(damage.nodes().is_empty());
        assert!(damage.rects().is_empty());

        // Later additions are ignored rather than resurrecting a partial path.
        damage.add_node(node(2));
        damage.add_rect(PipelineRect::new(0.0, 0.0, 10.0, 10.0));
        assert!(damage.nodes().is_empty());
        assert!(damage.rects().is_empty());
    }

    #[test]
    fn bounding_rect_unions_every_rect() {
        let mut damage = Damage::none();
        assert!(damage.bounding_rect().is_none());

        damage.escalate(DamageLevel::Paint);
        damage.add_rect(PipelineRect::new(10.0, 10.0, 20.0, 20.0));
        damage.add_rect(PipelineRect::new(50.0, 5.0, 10.0, 10.0));

        let union = damage.bounding_rect().expect("two rects were added");
        assert_eq!((union.x, union.y), (10.0, 5.0));
        assert_eq!((union.width, union.height), (50.0, 25.0));
    }

    #[test]
    fn take_resets_to_none() {
        let mut damage = Damage::none();
        damage.style_changed_paint_only(node(1));

        let taken = damage.take();
        assert_eq!(taken.level(), DamageLevel::Paint);
        assert_eq!(taken.nodes(), &[node(1)]);

        assert!(damage.is_none());
        assert!(damage.nodes().is_empty());
    }
}
