use gosub_render_backend::{
    Brush, Color, Point, Rect, RenderBackend, RenderRect, Scene, SizeU32, Transform,
};
use std::cmp::max;
use std::f32::consts::PI;

pub fn px_scale<B: RenderBackend>(size: SizeU32, offset: Point, width: f32) -> B::Scene {
    let mut scene = B::Scene::new();

    let len = max(
        size.width as i32 - offset.x as i32,
        size.height as i32 - offset.y as i32,
    ) as u32;

    let scale = draw_scale::<B>(len, 50);

    let transform = B::Transform::translate(offset.x, 0.0);

    scene.apply_scene(&scale, Some(transform));

    let transform = B::Transform::translate(width, offset.y).pre_rotate(PI / 2.0);

    scene.apply_scene(&scale, Some(transform));

    scene
}

pub fn draw_scale<B: RenderBackend>(len: u32, interval: u32) -> B::Scene {
    let mut scene = B::Scene::new();

    let mut x = 0;

    while x < len {
        let mut height = 50.0;

        if x % 100 == 0 {
            height = 60.0;
        }

        scene.draw_rect(&RenderRect {
            rect: B::Rect::new(x as f32, 0.0, 2.0, height),
            transform: None,
            radius: None,
            brush: B::Brush::color(Color::BLACK),
            brush_transform: None,
            border: None,
        });

        scene.debug_draw_simple_text(&format!("{}", x), Point::new(x as f32, height + 10.0), 12.0);
        x += interval;
    }

    scene
}
