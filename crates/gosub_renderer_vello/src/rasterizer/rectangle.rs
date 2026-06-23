use crate::rasterizer::brush::set_brush;
use gosub_render_pipeline::common::media::MediaStore;
use gosub_render_pipeline::painter::commands::border::BorderStyle;
use gosub_render_pipeline::painter::commands::rectangle::Rectangle;
use vello::kurbo;
use vello::kurbo::{Affine, PathEl, Point, Rect, RoundedRect, Shape};
use vello::peniko::Fill;

pub(crate) fn do_paint_rectangle(scene: &mut vello::Scene, rect: &Rectangle, affine: Affine, media_store: &MediaStore) {
    if let Some(brush) = rect.background() {
        let vello_rect = setup_rectangle_path(rect);
        let vello_brush = set_brush(brush, rect.rect(), media_store);
        scene.fill(Fill::NonZero, affine, &vello_brush, None, &vello_rect);
    }

    // Per-side borders (e.g. `border-bottom` only) are filled edge-by-edge.
    if !rect.border().is_uniform() {
        paint_per_side_border(scene, rect, affine, media_store);
        return;
    }

    match rect.border().style() {
        BorderStyle::None => {}
        BorderStyle::Solid => draw_single_border(scene, rect, affine, vec![], media_store),
        BorderStyle::Dashed => draw_single_border(scene, rect, affine, vec![50.0, 10.0, 10.0, 10.0], media_store),
        BorderStyle::Dotted => draw_single_border(scene, rect, affine, vec![10.0, 10.0], media_store),
        BorderStyle::Double => draw_double_border(scene, rect, affine, media_store),
        BorderStyle::Groove | BorderStyle::Ridge | BorderStyle::Inset | BorderStyle::Outset => {
            log::warn!(
                "Border style {:?} not yet implemented, falling back to solid",
                rect.border().style()
            );
            draw_single_border(scene, rect, affine, vec![], media_store)
        }
        BorderStyle::Hidden => {}
    }
}

/// Paints a non-uniform border by filling each visible side as a solid edge rectangle.
/// Side order is `[top, right, bottom, left]`.
fn paint_per_side_border(scene: &mut vello::Scene, rect: &Rectangle, affine: Affine, media_store: &MediaStore) {
    let r = rect.rect();
    let widths = rect.border().widths();
    let styles = rect.border().styles();
    let brushes = rect.border().brushes();

    let edges = [
        (r.x, r.y, r.width, widths[0] as f64),
        (r.x + r.width - widths[1] as f64, r.y, widths[1] as f64, r.height),
        (r.x, r.y + r.height - widths[2] as f64, r.width, widths[2] as f64),
        (r.x, r.y, widths[3] as f64, r.height),
    ];

    for i in 0..4 {
        if widths[i] <= 0.0 || styles[i].is_invisible() {
            continue;
        }
        let (x, y, w, h) = edges[i];
        let vello_brush = set_brush(&brushes[i], r, media_store);
        let edge = Rect::new(x, y, x + w, y + h);
        scene.fill(Fill::NonZero, affine, &vello_brush, None, &edge);
    }
}

fn draw_single_border(
    scene: &mut vello::Scene,
    rect: &Rectangle,
    affine: Affine,
    dashes: Vec<f64>,
    media_store: &MediaStore,
) {
    let binding = rect.border().brushes();
    let Some(brush) = binding.first() else {
        return;
    };
    let vello_shape = setup_rectangle_path(rect);
    let vello_brush = set_brush(brush, rect.rect(), media_store);
    let vello_stroke = kurbo::Stroke::new(rect.border().width() as f64).with_dashes(0.0, dashes);
    scene.stroke(&vello_stroke, affine, &vello_brush, None, &vello_shape);
}

fn draw_double_border(scene: &mut vello::Scene, rect: &Rectangle, affine: Affine, media_store: &MediaStore) {
    let binding = rect.border().brushes();
    let Some(brush) = binding.first() else {
        return;
    };
    let vello_shape = setup_rectangle_path(rect);
    let vello_brush = set_brush(brush, rect.rect(), media_store);

    if rect.border().width() < 3.0 {
        scene.stroke(
            &kurbo::Stroke::new(rect.border().width() as f64),
            affine,
            &vello_brush,
            None,
            &vello_shape,
        );
        return;
    }

    let width = (rect.border().width() / 2.0).floor();
    scene.stroke(
        &kurbo::Stroke::new(width as f64),
        affine,
        &vello_brush,
        None,
        &vello_shape,
    );

    let gap_size = 1.0;
    let inset = width as f64 + gap_size;

    let inner_border_shape = if rect.is_rounded() {
        let (r_tl, r_tr, r_br, r_bl) = rect.radius_x();
        ShapeEnum::RoundedRect(RoundedRect::new(
            rect.rect().x + inset,
            rect.rect().y + inset,
            rect.rect().x + rect.rect().width - inset,
            rect.rect().y + rect.rect().height - inset,
            (
                (r_tl - inset).max(0.0),
                (r_tr - inset).max(0.0),
                (r_br - inset).max(0.0),
                (r_bl - inset).max(0.0),
            ),
        ))
    } else {
        ShapeEnum::Rect(Rect::new(
            rect.rect().x + inset,
            rect.rect().y + inset,
            rect.rect().x + rect.rect().width - inset,
            rect.rect().y + rect.rect().height - inset,
        ))
    };
    scene.stroke(
        &kurbo::Stroke::new(width as f64),
        affine,
        &vello_brush,
        None,
        &inner_border_shape,
    );
}

enum ShapeEnum {
    Rect(Rect),
    RoundedRect(RoundedRect),
}

impl Shape for ShapeEnum {
    type PathElementsIter<'iter> = Box<dyn Iterator<Item = PathEl> + 'iter>;

    fn path_elements(&self, tolerance: f64) -> Self::PathElementsIter<'_> {
        match self {
            ShapeEnum::Rect(rect) => Box::new(rect.path_elements(tolerance)),
            ShapeEnum::RoundedRect(rounded_rect) => Box::new(rounded_rect.path_elements(tolerance)),
        }
    }

    fn area(&self) -> f64 {
        match self {
            ShapeEnum::Rect(rect) => rect.area(),
            ShapeEnum::RoundedRect(rounded_rect) => rounded_rect.area(),
        }
    }

    fn perimeter(&self, accuracy: f64) -> f64 {
        match self {
            ShapeEnum::Rect(rect) => rect.perimeter(accuracy),
            ShapeEnum::RoundedRect(rounded_rect) => rounded_rect.perimeter(accuracy),
        }
    }

    fn winding(&self, pt: Point) -> i32 {
        match self {
            ShapeEnum::Rect(rect) => rect.winding(pt),
            ShapeEnum::RoundedRect(rounded_rect) => rounded_rect.winding(pt),
        }
    }

    fn bounding_box(&self) -> Rect {
        match self {
            ShapeEnum::Rect(rect) => rect.bounding_box(),
            ShapeEnum::RoundedRect(rounded_rect) => rounded_rect.bounding_box(),
        }
    }
}

fn setup_rectangle_path(rect: &Rectangle) -> ShapeEnum {
    if rect.is_rounded() {
        let (r_tl, r_tr, r_br, r_bl) = rect.radius_x();
        return ShapeEnum::RoundedRect(RoundedRect::new(
            rect.rect().x,
            rect.rect().y,
            rect.rect().x + rect.rect().width,
            rect.rect().y + rect.rect().height,
            (r_tl, r_tr, r_br, r_bl),
        ));
    }

    ShapeEnum::Rect(Rect::new(
        rect.rect().x,
        rect.rect().y,
        rect.rect().x + rect.rect().width,
        rect.rect().y + rect.rect().height,
    ))
}
