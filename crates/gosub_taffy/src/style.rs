use taffy::Style;

use crate::Display;
use gosub_render_backend::layout::Node;

mod parse;
mod parse_properties;

const SCROLLBAR_WIDTH: f32 = 16.0;

pub fn get_style_from_node(node: &mut impl Node) -> (Style, Display) {
    //TODO: theoretically we should limit this to the taffy layouter, since it doesn't make any sense otherweise
    let (display, disp) = parse_properties::parse_display(node);
    let overflow = parse_properties::parse_overflow(node);
    let position = parse_properties::parse_position(node);
    let inset = parse_properties::parse_inset(node);
    let size = parse_properties::parse_size(node);
    let min_size = parse_properties::parse_min_size(node);
    let max_size = parse_properties::parse_max_size(node);
    let aspect_ratio = parse_properties::parse_aspect_ratio(node);
    let margin = parse_properties::parse_margin(node);
    let padding = parse_properties::parse_padding(node);
    let border = parse_properties::parse_border(node);
    let align_items = parse_properties::parse_align_items(node);
    let align_self = parse_properties::parse_align_self(node);
    let justify_items = parse_properties::parse_justify_items(node);
    let justify_self = parse_properties::parse_justify_self(node);
    let align_content = parse_properties::parse_align_content(node);
    let justify_content = parse_properties::parse_justify_content(node);
    let gap = parse_properties::parse_gap(node);
    let flex_direction = parse_properties::parse_flex_direction(node);
    let flex_wrap = parse_properties::parse_flex_wrap(node);
    let flex_basis = parse_properties::parse_flex_basis(node);
    let flex_grow = parse_properties::parse_flex_grow(node);
    let flex_shrink = parse_properties::parse_flex_shrink(node);
    let grid_template_rows = parse_properties::parse_grid_template_rows(node);
    let grid_template_columns = parse_properties::parse_grid_template_columns(node);
    let grid_auto_rows = parse_properties::parse_grid_auto_rows(node);
    let grid_auto_columns = parse_properties::parse_grid_auto_columns(node);
    let grid_auto_flow = parse_properties::parse_grid_auto_flow(node);
    let grid_row = parse_properties::parse_grid_row(node);
    let grid_column = parse_properties::parse_grid_column(node);

    (
        Style {
            display,
            overflow,
            scrollbar_width: SCROLLBAR_WIDTH,
            position,
            inset,
            size,
            min_size,
            max_size,
            aspect_ratio,
            margin,
            padding,
            border,
            align_items,
            align_self,
            justify_items,
            justify_self,
            align_content,
            justify_content,
            gap,
            flex_direction,
            flex_wrap,
            flex_basis,
            flex_grow,
            flex_shrink,
            grid_template_rows,
            grid_template_columns,
            grid_auto_rows,
            grid_auto_columns,
            grid_auto_flow,
            grid_row,
            grid_column,
        },
        disp,
    )
}
