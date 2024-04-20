use anyhow::anyhow;
use taffy::{AvailableSpace, PrintTree, Size};
use vello::kurbo::{Affine, Rect, RoundedRect};
use vello::peniko::{Color, Fill};
use vello::Scene;
use winit::dpi::PhysicalSize;

use gosub_styling::css_colors::RgbColor;
use gosub_styling::css_values::CssValue;
use gosub_styling::render_tree::RenderNodeData;

use crate::render_tree::{NodeID, TreeDrawer};

pub trait SceneDrawer {
    /// Returns true if the texture needs to be redrawn
    fn draw(&mut self, scene: &mut Scene, size: PhysicalSize<u32>) -> bool;
}

impl SceneDrawer for TreeDrawer {
    fn draw(&mut self, scene: &mut Scene, size: PhysicalSize<u32>) -> bool {
        if self.size == Some(size) {
            //This check needs to be updated in the future, when the tree is mutable
            return false;
        }

        self.size = Some(size);

        scene.reset();
        self.render(scene, size);
        true
    }
}

impl TreeDrawer {
    pub(crate) fn render(&mut self, scene: &mut Scene, size: PhysicalSize<u32>) {
        let space = Size {
            width: AvailableSpace::Definite(size.width as f32),
            height: AvailableSpace::Definite(size.height as f32),
        };

        if let Err(e) = self.taffy.compute_layout(self.root, space) {
            eprintln!("Failed to compute layout: {:?}", e);
            return;
        }

        let bg = Rect::new(0.0, 0.0, size.width as f64, size.height as f64);
        scene.fill(Fill::NonZero, Affine::IDENTITY, Color::BLACK, None, &bg);

        self.render_node_with_children(self.root, scene, (0.0, 0.0));
    }

    fn render_node_with_children(&mut self, id: NodeID, scene: &mut Scene, mut pos: (f64, f64)) {
        let err = self.render_node(id, scene, &mut pos);
        if let Err(e) = err {
            eprintln!("Error rendering node: {:?}", e);
        }

        let children = match self.taffy.children(id) {
            Ok(children) => children,
            Err(e) => {
                eprintln!("Error rendering node children: {e}");
                return;
            }
        };

        for child in children {
            self.render_node_with_children(child, scene, pos);
        }
    }

    fn render_node(
        &mut self,
        id: NodeID,
        scene: &mut Scene,
        pos: &mut (f64, f64),
    ) -> anyhow::Result<()> {
        let gosub_id = *self
            .taffy
            .get_node_context(id)
            .ok_or(anyhow!("Failed to get style id"))?;

        let layout = self.taffy.get_final_layout(id);

        let node = self
            .style
            .get_node_mut(gosub_id)
            .ok_or(anyhow!("Node not found"))?;

        pos.0 += layout.location.x as f64;
        pos.1 += layout.location.y as f64;

        if let RenderNodeData::Text(text) = &node.data {
            let color = node
                .properties
                .get("color")
                .and_then(|prop| {
                    prop.compute_value();

                    match &prop.actual {
                        CssValue::Color(color) => Some(*color),
                        CssValue::String(color) => Some(RgbColor::from(color.as_str())),
                        _ => None,
                    }
                })
                .map(|color| {
                    Color::rgba8(color.r as u8, color.g as u8, color.b as u8, color.a as u8)
                })
                .unwrap_or(Color::BLACK);

            let affine = Affine::translate((pos.0, pos.1));

            text.show(scene, color, affine, Fill::NonZero, None);
        }

        let bg_color = node
            .properties
            .get("background-color")
            .and_then(|prop| {
                prop.compute_value();

                match &prop.actual {
                    CssValue::Color(color) => Some(*color),
                    CssValue::String(color) => Some(RgbColor::from(color.as_str())),
                    _ => None,
                }
            })
            .map(|color| Color::rgba8(color.r as u8, color.g as u8, color.b as u8, color.a as u8));

        let border_radius = node
            .properties
            .get("border-radius")
            .map(|prop| prop.actual.unit_to_px() as f64)
            .unwrap_or(0.0);

        if let Some(bg_color) = bg_color {
            println!("Rendering background color: {:?}", bg_color);
            let rect = RoundedRect::new(
                pos.0,
                pos.1,
                layout.size.width as f64,
                layout.size.height as f64,
                border_radius,
            );
            scene.fill(Fill::NonZero, Affine::IDENTITY, bg_color, None, &rect);
        }

        Ok(())
    }
}
