use taffy::{
    AvailableSpace, CollapsibleMarginSet, Layout, LayoutInput, LayoutOutput, LayoutPartialTree,
    NodeId, Point, ResolveOrZero, RunMode, Size,
};

use gosub_render_backend::layout::LayoutTree;

use crate::{LayoutDocument, TaffyLayouter};

pub fn compute_inline_layout<LT: LayoutTree<TaffyLayouter>>(
    tree: &mut LayoutDocument<LT>,
    nod_id: LT::NodeId,
    mut layout_input: LayoutInput,
) -> LayoutOutput {
    layout_input.known_dimensions = Size::NONE;
    layout_input.run_mode = RunMode::PerformLayout; //TODO: We should respect the run mode

    let Some(children) = tree.0.children(nod_id) else {
        return LayoutOutput::HIDDEN;
    };

    let mut outputs = Vec::with_capacity(children.len());

    let mut height = 0.0f32;
    for child in &children {
        let node_id = NodeId::from((*child).into());

        let out = tree.compute_child_layout(node_id, layout_input);

        let style = tree.get_style(node_id);

        let margin = style.margin.resolve_or_zero(layout_input.parent_size);

        let child_height = out.size.height + margin.top + margin.bottom;

        height = height.max(child_height);

        outputs.push(out);
    }

    let mut width = 0.0f32;
    for (child, out) in children.into_iter().zip(outputs.into_iter()) {
        let node_id = NodeId::from(child.into());

        let style = tree.get_style(node_id);

        let location = Point {
            x: width,
            y: height - out.size.height,
        };

        let border = style.border.resolve_or_zero(layout_input.parent_size);
        let padding = style.padding.resolve_or_zero(layout_input.parent_size);

        width += out.size.width + border.left + border.right + padding.left + padding.right;

        tree.set_unrounded_layout(
            node_id,
            &Layout {
                size: out.size,
                content_size: out.content_size,
                order: 0,
                location,
                border,
                padding,
                scrollbar_size: Size::ZERO, //TODO
            },
        );
    }

    let content_size = Size { width, height };

    let mut size = content_size;

    if let AvailableSpace::Definite(width) = layout_input.available_space.width {
        size.width = size.width.min(width);
    }

    if let AvailableSpace::Definite(height) = layout_input.available_space.height {
        size.height = size.height.min(height);
    }

    LayoutOutput {
        size,
        content_size,
        first_baselines: Point::NONE,
        top_margin: CollapsibleMarginSet::ZERO,
        bottom_margin: CollapsibleMarginSet::ZERO,
        margins_can_collapse_through: false,
    }
}
