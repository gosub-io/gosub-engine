use gosub_render_backend::{RenderBackend, RenderRect, RenderText, Scene as TScene};
use vello::kurbo::RoundedRect;
use vello::peniko::Fill;
use vello::Scene as VelloScene;

use crate::{Border, BorderRenderOptions, Text, Transform, VelloBackend};

pub struct Scene(pub(crate) VelloScene);

impl TScene<VelloBackend> for Scene {
    fn reset(&mut self) {
        self.0.reset()
    }

    fn draw_rect(&mut self, rect: &RenderRect<VelloBackend>) {
        let affine = rect.transform.as_ref().map(|t| t.0).unwrap_or_default();

        let brush = &rect.brush.0;
        let brush_transform = rect.brush_transform.as_ref().map(|t| t.0);

        if let Some(radius) = &rect.radius {
            let shape = RoundedRect::from_rect(rect.rect.0, radius.clone());
            self.0
                .fill(Fill::NonZero, affine, brush, brush_transform, &shape)
        } else {
            self.0
                .fill(Fill::NonZero, affine, brush, brush_transform, &rect.rect.0)
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

    fn apply_scene(
        &mut self,
        scene: &<VelloBackend as RenderBackend>::Scene,
        transform: Option<Transform>,
    ) {
        self.0.append(&scene.0, transform.map(|t| t.0));
    }

    fn new(_data: &mut <VelloBackend as RenderBackend>::WindowData<'_>) -> Self {
        VelloScene::new().into()
    }
}

impl From<VelloScene> for Scene {
    fn from(scene: VelloScene) -> Self {
        Self(scene)
    }
}
