use std::fmt::{Debug, Formatter};
use vello::kurbo::RoundedRect;
use vello::peniko::Fill;
use vello::Scene as VelloScene;

use gosub_render_backend::{Point, RenderBackend, RenderRect, RenderText, Scene as TScene, FP};

use crate::debug::text::render_text_simple;
use crate::{Border, BorderRenderOptions, Text, Transform, VelloBackend};

#[derive(Clone)]
pub struct Scene(pub(crate) VelloScene);

impl Debug for Scene {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Scene").finish()
    }
    
}

impl Scene {
    pub fn inner(&mut self) -> &mut VelloScene {
        &mut self.0
    }

    pub fn create() -> Self {
        Self(VelloScene::new())
    }
}

impl TScene<VelloBackend> for Scene {
    fn draw_rect(&mut self, rect: &RenderRect<VelloBackend>) {
        let affine = rect.transform.as_ref().map(|t| t.0).unwrap_or_default();

        let brush = &rect.brush.0;
        let brush_transform = rect.brush_transform.as_ref().map(|t| t.0);

        if let Some(radius) = &rect.radius {
            let shape = RoundedRect::from_rect(rect.rect.0, radius.clone());
            self.0.fill(Fill::NonZero, affine, brush, brush_transform, &shape)
        } else {
            self.0.fill(Fill::NonZero, affine, brush, brush_transform, &rect.rect.0)
        }

        if let Some(border) = &rect.border {
            let opts = BorderRenderOptions {
                border,
                rect: &rect.rect,
                transform: rect.transform.as_ref(),
                radius: rect.radius.as_ref(),
            };

            Border::draw(&mut self.0, opts);
        }
    }

    fn draw_text(&mut self, text: &RenderText<VelloBackend>) {
        Text::show(&mut self.0, text)
    }

    fn debug_draw_simple_text(&mut self, text: &str, pos: Point, size: FP) {
        render_text_simple(self, text, pos, size)
    }

    fn apply_scene(&mut self, scene: &<VelloBackend as RenderBackend>::Scene, transform: Option<Transform>) {
        // let enc = self.0.encoding();
        //
        // enc.

        self.0.append(&scene.0, transform.map(|t| t.0));
    }

    fn reset(&mut self) {
        self.0.reset()
    }

    fn new() -> Self {
        VelloScene::new().into()
    }
}

impl From<VelloScene> for Scene {
    fn from(scene: VelloScene) -> Self {
        Self(scene)
    }
}
