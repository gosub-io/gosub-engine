use taffy::Style;

use crate::style::parse::CalcStorage;
use crate::Display;
use gosub_interface::config::HasLayouter;
use gosub_interface::layout::LayoutNode;

pub mod parse;
mod parse_properties;

const SCROLLBAR_WIDTH: f32 = 16.0;

// This function will convert a node into a Style object with Taffy properties.
//
// Returns the [`Style`], the gosub-side [`Display`] and the boxed `calc()` expressions referenced
// by the style. The caller must keep `CalcStorage` alive for as long as the style is in use,
// since the style contains raw pointers into those boxes.
pub fn get_style_from_node<C: HasLayouter>(node: &mut impl LayoutNode<C>) -> (Style, Display, CalcStorage) {
    //TODO: theoretically we should limit this to the taffy layouter, since it doesn't make any sense otherwise
    let mut calc_storage = CalcStorage::new();

    let (display, disp) = parse_properties::parse_display(node);
    let overflow = parse_properties::parse_overflow(node);
    let position = parse_properties::parse_position(node);
    let inset = parse_properties::parse_inset(node, &mut calc_storage);
    let size = parse_properties::parse_size(node, &mut calc_storage);
    let min_size = parse_properties::parse_min_size(node, &mut calc_storage);
    let max_size = parse_properties::parse_max_size(node, &mut calc_storage);
    let aspect_ratio = parse_properties::parse_aspect_ratio(node);
    let margin = parse_properties::parse_margin(node, &mut calc_storage);
    let padding = parse_properties::parse_padding(node, &mut calc_storage);
    let border = parse_properties::parse_border(node, &mut calc_storage);
    let align_items = parse_properties::parse_align_items(node);
    let align_self = parse_properties::parse_align_self(node);
    let justify_items = parse_properties::parse_justify_items(node);
    let justify_self = parse_properties::parse_justify_self(node);
    let align_content = parse_properties::parse_align_content(node);
    let justify_content = parse_properties::parse_justify_content(node);
    let gap = parse_properties::parse_gap(node, &mut calc_storage);
    let flex_direction = parse_properties::parse_flex_direction(node);
    let flex_wrap = parse_properties::parse_flex_wrap(node);
    let flex_basis = parse_properties::parse_flex_basis(node, &mut calc_storage);
    let flex_grow = parse_properties::parse_flex_grow(node);
    let flex_shrink = parse_properties::parse_flex_shrink(node);
    let grid_template_rows = parse_properties::parse_grid_template_rows(node);
    let grid_template_columns = parse_properties::parse_grid_template_columns(node);
    let grid_auto_rows = parse_properties::parse_grid_auto_rows(node);
    let grid_auto_columns = parse_properties::parse_grid_auto_columns(node);
    let grid_auto_flow = parse_properties::parse_grid_auto_flow(node);
    let grid_row = parse_properties::parse_grid_row(node);
    let grid_column = parse_properties::parse_grid_column(node);
    let box_sizing = parse_properties::parse_box_sizing(node);
    let text_align = parse_properties::parse_text_align(node);

    (
        Style {
            display,
            item_is_replaced: false,
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
            grid_template_areas: Vec::new(),
            grid_template_column_names: Vec::new(),
            grid_template_row_names: Vec::new(),
            grid_row,
            grid_column,
            // item_is_table: disp == Display::Table,
            item_is_table: false,
            box_sizing,
            text_align,
            ..Style::default()
        },
        disp,
        calc_storage,
    )
}
