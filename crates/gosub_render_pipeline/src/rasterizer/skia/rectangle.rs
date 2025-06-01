use skia_safe::Vector;
use skia_safe::Paint as SkiaPaint;
use crate::common::geo::Rect;
use crate::painter::commands::border::BorderStyle;
use crate::painter::commands::rectangle::Rectangle;
use crate::rasterizer::skia::paint::{create_paint, Paint};
use crate::tiler::Tile;

pub(crate) fn do_paint_rectangle(canvas: &skia_safe::Canvas, _tile: &Tile, rect: &Rectangle) {
    // Draw background (if any background brush is defined)
    match rect.background() {
        Some(brush) => {
            let shape = create_rect_shape(rect, None);
            let mut skia_paint = create_paint(brush);
            skia_paint.paint_mut().set_style(skia_safe::PaintStyle::Fill);

            shape.draw(canvas, &skia_paint);
        }
        None => {}
    }

    // Create border
    match rect.border().style() {
        BorderStyle::None => {},
        BorderStyle::Solid => draw_single_border(canvas, rect, vec![]),
        BorderStyle::Dashed => draw_single_border(canvas, rect, vec![50.0, 10.0, 10.0, 10.0]),
        BorderStyle::Dotted => draw_single_border(canvas, rect, vec![10.0, 10.0]),
        BorderStyle::Double => draw_double_border(canvas, rect, vec![]),
        BorderStyle::Groove => { unimplemented!() }
        BorderStyle::Ridge => { unimplemented!() }
        BorderStyle::Inset => { unimplemented!() }
        BorderStyle::Outset => { unimplemented!() }
        BorderStyle::Hidden => {
            // Don't display anything. But the border still takes up space. This is already
            // calculated in the box model by the layouter.
        }
    }
}

fn draw_single_border(canvas: &skia_safe::Canvas, rect: &Rectangle, dashes: Vec<f64>) {
    let mut skia_paint = create_paint(&rect.border().brushes()[0]);
    skia_paint.paint_mut().set_style(skia_safe::PaintStyle::Stroke);
    skia_paint.paint_mut().set_stroke_width(rect.border().width());
    if !dashes.is_empty() {
        let dashes = dashes.iter().map(|x| *x as f32).collect::<Vec<f32>>();
        skia_paint.paint_mut().set_path_effect(skia_safe::PathEffect::dash(&dashes, 0.0));
    }

    let shape = create_rect_shape(rect, Some(1.0));
    println!("DrawRect (border): {:?}", shape.rect());
    shape.draw(canvas, &skia_paint);
}

fn draw_double_border(canvas: &skia_safe::Canvas, rect: &Rectangle, dashes: Vec<f64>) {
    let mut skia_paint = create_paint(&rect.border().brushes()[0]);
    skia_paint.paint_mut().set_stroke(true);
    skia_paint.paint_mut().set_stroke_width(rect.border().width());
    skia_paint.paint_mut().set_stroke_cap(skia_safe::PaintCap::Round);
    if !dashes.is_empty() {
        let dashes = dashes.iter().map(|x| *x as f32).collect::<Vec<f32>>();
        skia_paint.paint_mut().set_path_effect(skia_safe::PathEffect::dash(&dashes, 0.0));
    }

    let shape = create_rect_shape(rect, None);

    if rect.border().width() < 3.0 {
        // When the width is less than 3.0, we just draw a single line as there is no room for
        // a double border
        println!("DrawRect (dborder): {:?}", shape.rect());
        shape.draw(canvas, &skia_paint);
        return;
    }

    // The formula: outer border: (N-1) / 2, 1px gap, inner border: (N-1) / 2

    // Outer border
    let width = (rect.border().width() / 2.0).floor();
    skia_paint.paint_mut().set_stroke_width(width);
    println!("DrawRect (dborder): {:?}", shape.rect());
    shape.draw(canvas, &skia_paint);

    let gap_size = 1.0;

    // inner border
    let inner_border_rect = Rectangle::new(Rect::new(
        rect.rect().x + width as f64 + gap_size,
        rect.rect().y + width as f64 + gap_size,
        rect.rect().width - width as f64 - gap_size,
        rect.rect().height - width as f64 - gap_size
    ));
    let shape = create_rect_shape(&inner_border_rect, None);
    let skia_paint = create_paint(&rect.border().brushes()[0]);
    println!("DrawRect (dborder): {:?}", shape.rect());
    shape.draw(canvas, &skia_paint);
}

enum ShapeEnum {
    Rect(skia_safe::Rect),
    RoundedRect(skia_safe::RRect),
}

impl ShapeEnum {
    fn draw(&self, canvas: &skia_safe::Canvas, paint: &Paint) {
        // Fetch the rect dimensions
        let rect = self.rect();

        // Since skia can't scale automatically, we need to do this manually. This is why
        // we have a custom SkiaPaint enum that can handle this, as we cannot fetch the dimensions
        // of the image from skia's paint object.
        let skia_paint = match paint {
            Paint::Image(ip) => {
                if let Some(image_filter) = paint.paint().image_filter() {
                    let sx = rect.width / ip.dimension.width;
                    let sy = rect.height / ip.dimension.height;

                    // We scale the image from the top-left corner. We must translate the image to the correct
                    // position after scaling
                    let mut matrix = skia_safe::Matrix::default();
                    matrix.set_scale((sx as f32, sy as f32), None);
                    matrix.set_translate_x(rect.x as f32);
                    matrix.set_translate_y(rect.y as f32);

                    let scaled_image_filter = image_filter.with_local_matrix(&matrix);
                    let mut p = SkiaPaint::default();
                    p.set_image_filter(scaled_image_filter);
                    p
                } else {
                    paint.paint().clone()
                }
            },
            // Non-images just use the paint as is
            _ => paint.paint().clone(),
        };


        match self {
            ShapeEnum::Rect(rect) => {
                canvas.draw_rect(rect, skia_paint.as_ref());
            },
            ShapeEnum::RoundedRect(rrect) => {
                canvas.draw_rrect(rrect, skia_paint.as_ref());
            },
        }
    }

    fn rect(&self) -> Rect {
        match self {
            ShapeEnum::Rect(rect) => {
                Rect::new(
                    rect.left as f64,
                    rect.top as f64,
                    rect.width() as f64,
                    rect.height() as f64
                )
            },
            ShapeEnum::RoundedRect(rrect) => {
                Rect::new(
                    rrect.rect().left as f64,
                    rrect.rect().top as f64,
                    rrect.rect().width() as f64,
                    rrect.rect().height() as f64
                )
            },
        }
    }
}

fn create_rect_shape(rect: &Rectangle, _round: Option<f64>) -> ShapeEnum {
    let skia_rect = skia_safe::Rect::new(
        rect.rect().x as f32,
        rect.rect().y as f32,
        (rect.rect().x + rect.rect().width) as f32,
        (rect.rect().y + rect.rect().height) as f32,
    );

    if !rect.is_rounded() {
        return ShapeEnum::Rect(skia_rect);
    }

    let (r_tl, r_tr, r_br, r_bl) = rect.radius();
    let skia_rrect = skia_safe::RRect::new_rect_radii(
        skia_rect,
        &[
            Vector::new(r_tl.x as f32, r_tl.y as f32),
            Vector::new(r_tr.x as f32, r_tr.y as f32),
            Vector::new(r_br.x as f32, r_br.y as f32),
            Vector::new(r_bl.x as f32, r_bl.y as f32)
        ],
    );
    ShapeEnum::RoundedRect(skia_rrect)
}