use crate::rasterizer::brush::set_brush;
use gosub_render_pipeline::common::media::MediaStore;
use gosub_render_pipeline::painter::commands::border::{per_side_strips, BorderStyle};
use gosub_render_pipeline::painter::commands::rectangle::{BlendMode, Rectangle};
use vello::kurbo;
use vello::kurbo::{Affine, PathEl, Point, Rect, RoundedRect, Shape};
use vello::peniko::{Fill, Mix};

/// CSS `mix-blend-mode` → Vello mix mode. Applied by wrapping the element's drawing in a
/// blend layer, which composites it against the scene content painted beneath it.
fn to_vello_mix(mode: BlendMode) -> Mix {
    match mode {
        BlendMode::Normal => Mix::Normal,
        BlendMode::Multiply => Mix::Multiply,
        BlendMode::Screen => Mix::Screen,
        BlendMode::Overlay => Mix::Overlay,
        BlendMode::Darken => Mix::Darken,
        BlendMode::Lighten => Mix::Lighten,
        BlendMode::ColorDodge => Mix::ColorDodge,
        BlendMode::ColorBurn => Mix::ColorBurn,
        BlendMode::HardLight => Mix::HardLight,
        BlendMode::SoftLight => Mix::SoftLight,
        BlendMode::Difference => Mix::Difference,
        BlendMode::Exclusion => Mix::Exclusion,
        BlendMode::Hue => Mix::Hue,
        BlendMode::Saturation => Mix::Saturation,
        BlendMode::Color => Mix::Color,
        BlendMode::Luminosity => Mix::Luminosity,
    }
}

pub(crate) fn do_paint_rectangle(scene: &mut vello::Scene, rect: &Rectangle, affine: Affine, media_store: &MediaStore) {
    // Vello fills carry no per-draw blend mode; a non-normal mix-blend-mode wraps the whole
    // rectangle (background + borders) in a blend layer clipped to the border box, outset by
    // the border width since strokes are centred on the path.
    let blended = rect.blend_mode() != BlendMode::Normal;
    if blended {
        let r = rect.rect();
        let outset = rect.border().width() as f64;
        let clip = Rect::new(
            r.x - outset,
            r.y - outset,
            r.x + r.width + outset,
            r.y + r.height + outset,
        );
        scene.push_layer(Fill::NonZero, to_vello_mix(rect.blend_mode()), 1.0, affine, &clip);
    }
    paint_rectangle_content(scene, rect, affine, media_store);
    if blended {
        scene.pop_layer();
    }
}

fn paint_rectangle_content(scene: &mut vello::Scene, rect: &Rectangle, affine: Affine, media_store: &MediaStore) {
    if let Some(brush) = rect.background() {
        let vello_rect = setup_rectangle_path(rect);
        let (vello_brush, brush_transform) = set_brush(brush, rect.rect(), media_store);
        scene.fill(Fill::NonZero, affine, &vello_brush, brush_transform, &vello_rect);
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
    let brushes = rect.border().brushes();
    let strips = per_side_strips(r, rect.border().widths(), &rect.border().styles());

    for (i, side) in strips.iter().enumerate() {
        if side.is_empty() {
            continue;
        }
        let (vello_brush, brush_transform) = set_brush(&brushes[i], r, media_store);
        for strip in side {
            let edge = Rect::new(strip.x, strip.y, strip.x + strip.width, strip.y + strip.height);
            scene.fill(Fill::NonZero, affine, &vello_brush, brush_transform, &edge);
        }
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
    let width = rect.border().width() as f64;
    let vello_shape = setup_inset_path(rect, width / 2.0);
    let (vello_brush, brush_transform) = set_brush(brush, rect.rect(), media_store);
    let vello_stroke = kurbo::Stroke::new(width).with_dashes(0.0, dashes);
    scene.stroke(&vello_stroke, affine, &vello_brush, brush_transform, &vello_shape);
}

fn draw_double_border(scene: &mut vello::Scene, rect: &Rectangle, affine: Affine, media_store: &MediaStore) {
    let binding = rect.border().brushes();
    let Some(brush) = binding.first() else {
        return;
    };
    let (vello_brush, brush_transform) = set_brush(brush, rect.rect(), media_store);

    if rect.border().width() < 3.0 {
        let width = rect.border().width() as f64;
        scene.stroke(
            &kurbo::Stroke::new(width),
            affine,
            &vello_brush,
            brush_transform,
            &setup_inset_path(rect, width / 2.0),
        );
        return;
    }

    // Two strands inside the border box, split in thirds so strands + gap never
    // exceed the declared width (the gap absorbs the rounding remainder).
    let total = rect.border().width() as f64;
    let strand = (total / 3.0).floor();
    let gap = total - 2.0 * strand;
    scene.stroke(
        &kurbo::Stroke::new(strand),
        affine,
        &vello_brush,
        brush_transform,
        &setup_inset_path(rect, strand / 2.0),
    );
    scene.stroke(
        &kurbo::Stroke::new(strand),
        affine,
        &vello_brush,
        brush_transform,
        &setup_inset_path(rect, strand + gap + strand / 2.0),
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
    setup_inset_path(rect, 0.0)
}

/// Builds the rectangle path inset by `inset` on every side. Strokes are
/// centered on the path, so a border of width `w` must stroke a path inset by
/// `w / 2` to stay entirely inside the border box (CSS borders never paint
/// outside it - a centered stroke bleeds into the neighbouring element and
/// gets painted over, which visibly halves adjacent borders).
fn setup_inset_path(rect: &Rectangle, inset: f64) -> ShapeEnum {
    let x0 = rect.rect().x + inset;
    let y0 = rect.rect().y + inset;
    let x1 = (rect.rect().x + rect.rect().width - inset).max(x0);
    let y1 = (rect.rect().y + rect.rect().height - inset).max(y0);

    if rect.is_rounded() {
        let (r_tl, r_tr, r_br, r_bl) = rect.radius_x();
        return ShapeEnum::RoundedRect(RoundedRect::new(
            x0,
            y0,
            x1,
            y1,
            (
                (r_tl - inset).max(0.0),
                (r_tr - inset).max(0.0),
                (r_br - inset).max(0.0),
                (r_bl - inset).max(0.0),
            ),
        ));
    }

    ShapeEnum::Rect(Rect::new(x0, y0, x1, y1))
}
