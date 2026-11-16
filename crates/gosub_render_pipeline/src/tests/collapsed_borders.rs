//! Collapsed table borders: the cells paint every border of the table, each segment in the
//! style of the border that won it (CSS 2 §17.6.2.1).

use std::sync::Arc;

use gosub_css3::system::Css3System;
use gosub_fontmanager::ParleyFontSystem;
use gosub_html5::document::document_impl::DocumentImpl;
use gosub_html5::html_compile;
use gosub_html5::parser::Html5Parser;
use gosub_interface::config::ModuleConfiguration;
use gosub_interface::css3::CssSystem as _;
use gosub_interface::document::Document as _;
use gosub_interface::font_system::FontSystem;
use gosub_shared::node::NodeId;
use parking_lot::Mutex;

use crate::common::browser_state::{BrowserState, WireframeState};
use crate::common::document::pipeline_doc::GosubDocumentAdapter;
use crate::common::geo::{Dimension, Rect};
use crate::common::media::MediaStore;
use crate::layering::layer::LayerList;
use crate::layouter::taffy::TaffyLayouter;
use crate::layouter::{CanLayout as _, ElementContext, LayoutElementId};
use crate::painter::commands::border::BorderStyle;
use crate::painter::commands::brush::Brush;
use crate::painter::commands::rectangle::Rectangle;
use crate::painter::commands::PaintCommand;
use crate::painter::Painter;
use crate::rendertree_builder::tree::RenderTree;

#[derive(Clone, Debug, PartialEq)]
struct Config;

impl ModuleConfiguration for Config {
    type CssSystem = Css3System;
    type Document = DocumentImpl<Self>;
    type HtmlParser = Html5Parser<'static, Self>;
}

/// `html` laid out in an 800x600 viewport, as a layer list, with the layout element of `id`.
fn lay_out(html: &str, id: &str) -> (Arc<LayerList>, LayoutElementId) {
    let mut doc = html_compile::<Config>(html);
    doc.add_stylesheet(Css3System::load_default_useragent_stylesheet());
    let adapter = GosubDocumentAdapter::<Config>::new(Arc::new(doc));
    fn find(a: &GosubDocumentAdapter<Config>, node: NodeId, id: &str) -> Option<NodeId> {
        if a.doc.attribute(node, "id") == Some(id) {
            return Some(node);
        }
        a.doc.children(node).iter().find_map(|&child| find(a, child, id))
    }
    let node = find(&adapter, adapter.doc.root(), id).unwrap_or_else(|| panic!("no element with id {id}"));
    let mut render_tree = RenderTree::new(Arc::new(adapter));
    render_tree.parse().expect("render tree");
    let font_system: Arc<Mutex<dyn FontSystem>> = Arc::new(Mutex::new(ParleyFontSystem::new()));
    let mut layouter = TaffyLayouter::with_font_system_and_media_store(font_system, Arc::new(MediaStore::new()));
    let tree = layouter.layout(render_tree, Some(Dimension::new(800.0, 600.0)), 1.0);
    let element = tree
        .arena
        .values()
        .find(|el| el.dom_node_id == node && !matches!(el.context, ElementContext::TableBorderOverlay(_)))
        .expect("the element is laid out")
        .id;
    (Arc::new(LayerList::new(Arc::new(tree))), element)
}

fn state() -> BrowserState {
    BrowserState {
        visible_layer_list: vec![true; 8],
        wireframed: WireframeState::None,
        debug_hover: false,
        current_hovered_element: None,
        show_tilegrid: false,
        debug_table_cells: false,
        viewport: Rect::new(0.0, 0.0, 800.0, 600.0),
        tile_list: None,
        dpi_scale_factor: 1.0,
    }
}

fn rectangles(commands: Vec<PaintCommand>) -> Vec<Rectangle> {
    commands
        .into_iter()
        .filter_map(|cmd| match cmd {
            PaintCommand::Rectangle(r) => Some(r),
            _ => None,
        })
        .collect()
}

fn is_red(r: &Rectangle) -> bool {
    r.border()
        .brushes()
        .iter()
        .any(|b| matches!(b, Brush::Solid(c) if c.r8() == 255 && c.g8() == 0 && c.b8() == 0))
}

/// A row's wider border wins the line under it and is painted by the cells on both sides; the
/// row itself paints none. Between equally wide borders, solid beats dashed.
#[test]
fn a_row_border_is_painted_by_the_cells() {
    let (layers, table) = lay_out(
        r#"<html><body><table id="t" style="border-collapse: collapse">
          <tr id="r" style="border-bottom: 5px solid rgb(255, 0, 0)">
            <td style="border: 1px solid blue">a</td>
            <td style="border: 1px solid blue; border-left-style: dashed">b</td>
          </tr>
          <tr><td style="border: 1px solid blue">c</td><td style="border: 1px solid blue">d</td></tr>
        </table></body></html>"#,
        "t",
    );
    let tree = &layers.layout_tree;
    let painter = Painter::new(Arc::clone(&layers), None);
    let table_el = tree.get_node_by_id(table).unwrap();
    let overlay = table_el
        .children
        .iter()
        .copied()
        .find(|&id| {
            matches!(
                tree.get_node_by_id(id).unwrap().context,
                ElementContext::TableBorderOverlay(_)
            )
        })
        .expect("a collapsed table has a border overlay");
    let strips = rectangles(painter.paint_element(overlay, &state()));

    let red: Vec<&Rectangle> = strips.iter().filter(|r| is_red(r)).collect();
    assert!(!red.is_empty(), "the row's border is painted: {strips:?}");
    let top = red.iter().map(|r| r.rect().y).fold(f64::MAX, f64::min);
    let bottom = red
        .iter()
        .map(|r| r.rect().y + r.rect().height)
        .fold(f64::MIN, f64::max);
    assert_eq!(bottom - top, 5.0, "both halves of the 5px line");
    let left = red.iter().map(|r| r.rect().x).fold(f64::MAX, f64::min);
    let right = red.iter().map(|r| r.rect().x + r.rect().width).fold(f64::MIN, f64::max);
    assert!(
        right - left >= table_el.box_model.border_box.width - 1.0,
        "the line runs the width of the table, {left}..{right}"
    );
    assert!(
        strips
            .iter()
            .all(|r| !r.border().styles().contains(&BorderStyle::Dashed)),
        "b's dashed left border loses to a's solid right one"
    );

    // The row, its group and the table paint no border of their own.
    for id in tree.arena.keys().copied() {
        let el = tree.get_node_by_id(id).unwrap();
        if el.collapsed_borders.is_some() || id == overlay {
            continue;
        }
        let painted = rectangles(painter.paint_element(id, &state()));
        assert!(
            painted.iter().all(|r| !is_red(r)),
            "only the overlay paints the row's border, {id:?} did: {painted:?}"
        );
    }
}

/// A row's border runs on over the slots no cell covers: the row paints them.
#[test]
fn a_row_border_runs_over_empty_slots() {
    let (layers, table) = lay_out(
        r#"<html><body><table id="t" style="border-collapse: collapse">
          <tr><td>a</td><td>b</td><td>c</td></tr>
          <tr style="border-bottom: 4px solid rgb(255, 0, 0)"><td>d</td></tr>
        </table></body></html>"#,
        "t",
    );
    let tree = &layers.layout_tree;
    let painter = Painter::new(Arc::clone(&layers), None);
    let table_el = tree.get_node_by_id(table).unwrap();
    let overlay = table_el
        .children
        .iter()
        .copied()
        .find(|&id| {
            matches!(
                tree.get_node_by_id(id).unwrap().context,
                ElementContext::TableBorderOverlay(_)
            )
        })
        .expect("a collapsed table has a border overlay");
    let red: Vec<Rectangle> = rectangles(painter.paint_element(overlay, &state()))
        .into_iter()
        .filter(is_red)
        .collect();
    let left = red.iter().map(|r| r.rect().x).fold(f64::MAX, f64::min);
    let right = red.iter().map(|r| r.rect().x + r.rect().width).fold(f64::MIN, f64::max);
    assert!(
        right - left >= table_el.box_model.border_box.width - 1.0,
        "the line runs the width of the table, {left}..{right}: {red:?}"
    );
}

/// A border thinner than a device pixel still paints one; both its halves used to round onto
/// the line between the cells, and nothing was drawn.
#[test]
fn a_thin_border_paints_a_device_pixel() {
    let (layers, table) = lay_out(
        r#"<html><body><table id="t" style="border-collapse: collapse">
          <tr><td style="width: 19.75px; padding: 0; border-right: 0.5px solid rgb(255, 0, 0)">a</td><td>b</td></tr>
        </table></body></html>"#,
        "t",
    );
    let tree = &layers.layout_tree;
    let painter = Painter::new(Arc::clone(&layers), None);
    let overlay = tree
        .get_node_by_id(table)
        .unwrap()
        .children
        .iter()
        .copied()
        .find(|&id| {
            matches!(
                tree.get_node_by_id(id).unwrap().context,
                ElementContext::TableBorderOverlay(_)
            )
        })
        .expect("a collapsed table has a border overlay");
    let red: Vec<Rectangle> = rectangles(painter.paint_element(overlay, &state()))
        .into_iter()
        .filter(is_red)
        .collect();
    let width: f64 = red.iter().map(|r| r.rect().width).sum();
    assert_eq!(width, 1.0, "{red:?}");
}
