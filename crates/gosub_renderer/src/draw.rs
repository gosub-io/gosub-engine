use std::io::Read;
use std::ops::Div;

use anyhow::anyhow;
use smallvec::SmallVec;
use taffy::{AvailableSpace, Layout, NodeId, PrintTree, Size, TaffyTree, TraversePartialTree};
use url::Url;
use vello::kurbo::{Affine, Arc, BezPath, Cap, Join, Rect, RoundedRect, Stroke};
use vello::peniko::{Color, Fill, Format, Image};
use vello::Scene;
use winit::dpi::PhysicalSize;

use gosub_css3::colors::RgbColor;
use gosub_css3::stylesheet::CssValue;
use gosub_html5::node::NodeId as GosubId;
use gosub_styling::render_tree::{RenderNodeData, RenderTree, RenderTreeNode};

use crate::render_tree::{NodeID, TreeDrawer};

pub trait SceneDrawer {
    /// Returns true if the texture needs to be redrawn
    fn draw(&mut self, scene: &mut Scene, size: PhysicalSize<u32>);
}

impl SceneDrawer for TreeDrawer {
    fn draw(&mut self, scene: &mut Scene, size: PhysicalSize<u32>) {
        if self.size == Some(size) {
            //This check needs to be updated in the future, when the tree is mutable
            return;
        }

        self.size = Some(size);

        scene.reset();
        self.render(scene, size);
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

        print_tree(&self.taffy, self.root, &self.style);

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

        let border_radius = render_bg(node, scene, layout, pos, &self.url);

        if let RenderNodeData::Element(element) = &node.data {
            if element.name() == "img" {
                let src = element
                    .attributes
                    .get("src")
                    .ok_or(anyhow!("Image element has no src attribute"))?;

                let url = Url::parse(src.as_str()).or_else(|_| self.url.join(src.as_str()))?;

                let res = gosub_net::http::ureq::get(url.as_str()).call()?;

                let mut img = Vec::with_capacity(
                    res.header("Content-Length")
                        .unwrap_or("1024")
                        .parse::<usize>()?,
                );

                res.into_reader().read_to_end(&mut img)?;

                let img = image::load_from_memory(&img)?;

                let width = img.width();
                let height = img.height();

                let img = Image::new(
                    img.into_rgba8().into_raw().into(),
                    Format::Rgba8,
                    width,
                    height,
                );

                let fit = element
                    .attributes
                    .get("object-fit")
                    .map(|prop| prop.as_str())
                    .unwrap_or("contain");

                render_image(&img, scene, *pos, layout.size, border_radius, fit)?;
            }
        }

        render_text(node, scene, pos, layout);

        render_border(node, scene, layout, pos, border_radius);

        Ok(())
    }
}

fn render_text(node: &mut RenderTreeNode, scene: &mut Scene, pos: &(f64, f64), layout: &Layout) {
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
            .map(|color| Color::rgba8(color.r as u8, color.g as u8, color.b as u8, color.a as u8))
            .unwrap_or(Color::BLACK);

        let affine = Affine::translate((pos.0, pos.1 + layout.size.height as f64));

        text.show(scene, color, affine, Fill::NonZero, None);
    }
}

fn render_bg(
    node: &mut RenderTreeNode,
    scene: &mut Scene,
    layout: &Layout,
    pos: &(f64, f64),
    root_url: &Url,
) -> (f64, f64, f64, f64) {
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

    let border_radius_left = node
        .properties
        .get("border-radius-left")
        .map(|prop| {
            prop.compute_value();
            prop.actual.unit_to_px() as f64
        })
        .unwrap_or(0.0);

    let border_radius_right = node
        .properties
        .get("border-radius-right")
        .map(|prop| {
            prop.compute_value();
            prop.actual.unit_to_px() as f64
        })
        .unwrap_or(0.0);

    let border_radius_top = node
        .properties
        .get("border-radius-top")
        .map(|prop| {
            prop.compute_value();
            prop.actual.unit_to_px() as f64
        })
        .unwrap_or(0.0);

    let border_radius_bottom = node
        .properties
        .get("border-radius-bottom")
        .map(|prop| {
            prop.compute_value();
            prop.actual.unit_to_px() as f64
        })
        .unwrap_or(0.0);

    let border_radius = (
        border_radius_top,
        border_radius_right,
        border_radius_bottom,
        border_radius_left,
    );

    let rect = RoundedRect::new(
        pos.0,
        pos.1,
        pos.0 + layout.size.width as f64,
        pos.1 + layout.size.height as f64,
        border_radius,
    );

    if let Some(bg_color) = bg_color {
        scene.fill(Fill::NonZero, Affine::IDENTITY, bg_color, None, &rect);
    }

    let background_image = node.properties.get("background-image").and_then(|prop| {
        prop.compute_value();

        match &prop.actual {
            CssValue::String(url) => Some(url.as_str()),
            _ => None,
        }
    });

    if let Some(url) = background_image {
        let Ok(url) = Url::parse(url).or_else(|_| root_url.join(url)) else {
            eprintln!("TODO: Add Image not found Image");
            return border_radius;
        };

        let res = gosub_net::http::ureq::get(url.as_str()).call().unwrap();

        let mut img = Vec::with_capacity(
            res.header("Content-Length")
                .unwrap_or("1024")
                .parse::<usize>()
                .unwrap(),
        );

        res.into_reader().read_to_end(&mut img).unwrap();

        let img = image::load_from_memory(&img).unwrap();

        let height = img.height();
        let width = img.width();

        let img = Image::new(
            img.into_rgba8().into_raw().into(),
            Format::Rgba8,
            width,
            height,
        );

        let _ = render_image(&img, scene, *pos, layout.size, border_radius, "fill").map_err(|e| {
            eprintln!("Error rendering image: {:?}", e);
        });
    }

    border_radius
}

enum Side {
    Top,
    Right,
    Bottom,
    Left,
}

impl Side {
    fn all() -> [Side; 4] {
        [Side::Top, Side::Right, Side::Bottom, Side::Left]
    }

    fn to_str(&self) -> &'static str {
        match self {
            Side::Top => "top",
            Side::Right => "right",
            Side::Bottom => "bottom",
            Side::Left => "left",
        }
    }
}

fn render_border(
    node: &mut RenderTreeNode,
    scene: &mut Scene,
    layout: &Layout,
    pos: &(f64, f64),
    border_radius: (f64, f64, f64, f64),
) {
    for side in Side::all() {
        let radi = match side {
            Side::Top => border_radius.0,
            Side::Right => border_radius.1,
            Side::Bottom => border_radius.2,
            Side::Left => border_radius.3,
        };
        render_border_side(node, scene, layout, pos, radi, side);
    }
}

fn render_border_side(
    node: &mut RenderTreeNode,
    scene: &mut Scene,
    layout: &Layout,
    pos: &(f64, f64),
    border_radius: f64,
    side: Side,
) {
    let border_width = match side {
        Side::Top => layout.border.top,
        Side::Right => layout.border.right,
        Side::Bottom => layout.border.bottom,
        Side::Left => layout.border.left,
    } as f64;

    let border_color = node
        .properties
        .get(&format!("border-{}-color", side.to_str()))
        .and_then(|prop| {
            prop.compute_value();

            match &prop.actual {
                CssValue::Color(color) => Some(*color),
                CssValue::String(color) => Some(RgbColor::from(color.as_str())),
                _ => None,
            }
        })
        .map(|color| Color::rgba8(color.r as u8, color.g as u8, color.b as u8, color.a as u8));

    // let border_radius = 16f64;

    let width = layout.size.width as f64;
    let height = layout.size.height as f64;

    if let Some(border_color) = border_color {
        let mut path = BezPath::new();

        //draw the border segment with rounded corners

        match side {
            Side::Top => {
                let offset = border_radius.powi(2).div(2.0).sqrt() - border_radius;

                path.move_to((pos.0 - offset, pos.1 - offset));

                let arc = Arc::new(
                    (pos.0 + border_radius, pos.1 + border_radius),
                    (border_radius, border_radius),
                    -std::f64::consts::PI * 3.0 / 4.0,
                    std::f64::consts::PI / 4.0,
                    0.0,
                );

                arc.to_cubic_beziers(0.1, |p1, p2, p3| {
                    path.curve_to(p1, p2, p3);
                });

                path.line_to((pos.0 + width - border_radius, pos.1));

                let arc = Arc::new(
                    (pos.0 + width - border_radius, pos.1 + border_radius),
                    (border_radius, border_radius),
                    -std::f64::consts::PI / 2.0,
                    std::f64::consts::PI / 4.0,
                    0.0,
                );

                arc.to_cubic_beziers(0.1, |p1, p2, p3| {
                    path.curve_to(p1, p2, p3);
                });
            }
            Side::Right => {
                let offset = border_radius.powi(2).div(2.0).sqrt() - border_radius;
                path.move_to((pos.0 + width + offset, pos.1 - offset));

                let arc = Arc::new(
                    (pos.0 + width - border_radius, pos.1 + border_radius),
                    (border_radius, border_radius),
                    -std::f64::consts::PI / 4.0,
                    std::f64::consts::PI / 4.0,
                    0.0,
                );

                arc.to_cubic_beziers(0.1, |p1, p2, p3| {
                    path.curve_to(p1, p2, p3);
                });

                path.line_to((pos.0 + width, pos.1 + height - border_radius));

                let arc = Arc::new(
                    (
                        pos.0 + width - border_radius,
                        pos.1 + height - border_radius,
                    ),
                    (border_radius, border_radius),
                    0.0,
                    std::f64::consts::PI / 4.0,
                    0.0,
                );

                arc.to_cubic_beziers(0.1, |p1, p2, p3| {
                    path.curve_to(p1, p2, p3);
                });
            }
            Side::Bottom => {
                let offset = border_radius.powi(2).div(2.0).sqrt() - border_radius;

                path.move_to((pos.0 + width + offset, pos.1 + height + offset));

                let arc = Arc::new(
                    (
                        pos.0 + width - border_radius,
                        pos.1 + height - border_radius,
                    ),
                    (border_radius, border_radius),
                    -std::f64::consts::PI * 7.0 / 4.0,
                    std::f64::consts::PI / 4.0,
                    0.0,
                );

                arc.to_cubic_beziers(0.1, |p1, p2, p3| {
                    path.curve_to(p1, p2, p3);
                });

                path.line_to((pos.0 + border_radius, pos.1 + height));

                let arc = Arc::new(
                    (pos.0 + border_radius, pos.1 + height - border_radius),
                    (border_radius, border_radius),
                    -std::f64::consts::PI * 3.0 / 2.0,
                    std::f64::consts::PI / 4.0,
                    0.0,
                );

                arc.to_cubic_beziers(0.1, |p1, p2, p3| {
                    path.curve_to(p1, p2, p3);
                });
            }
            Side::Left => {
                let offset = border_radius.powi(2).div(2.0).sqrt() - border_radius;

                path.move_to((pos.0 - offset, pos.1 + height + offset));

                let arc = Arc::new(
                    (pos.0 + border_radius, pos.1 + height - border_radius),
                    (border_radius, border_radius),
                    -std::f64::consts::PI * 5.0 / 4.0,
                    std::f64::consts::PI / 4.0,
                    0.0,
                );

                arc.to_cubic_beziers(0.1, |p1, p2, p3| {
                    path.curve_to(p1, p2, p3);
                });

                path.line_to((pos.0, pos.1 + border_radius));

                let arc = Arc::new(
                    (pos.0 + border_radius, pos.1 + border_radius),
                    (border_radius, border_radius),
                    -std::f64::consts::PI,
                    std::f64::consts::PI / 4.0,
                    0.0,
                );

                arc.to_cubic_beziers(0.1, |p1, p2, p3| {
                    path.curve_to(p1, p2, p3);
                });
            }
        }

        let Some(border_style) = node
            .properties
            .get(&format!("border-{}-style", side.to_str()))
            .and_then(|prop| {
                prop.compute_value();

                match &prop.actual {
                    CssValue::String(style) => Some(style.as_str()),
                    _ => None,
                }
            })
        else {
            return;
        };

        let border_style = BorderStyle::from_str(border_style);

        let cap = match border_style {
            BorderStyle::Dashed => Cap::Square,
            BorderStyle::Dotted => Cap::Round,
            _ => Cap::Butt,
        };

        let dash_pattern = match border_style {
            BorderStyle::Dashed => SmallVec::from([
                border_width * 3.0,
                border_width * 3.0,
                border_width * 3.0,
                border_width * 3.0,
            ]),
            BorderStyle::Dotted => {
                SmallVec::from([border_width, border_width, border_width, border_width])
                //TODO: somehow this doesn't result in circles. It is more like a rounded rectangle
            }
            _ => SmallVec::default(),
        };

        let stroke = Stroke {
            width: border_width,
            join: Join::Bevel,
            miter_limit: 0.0,
            start_cap: cap,
            end_cap: cap,
            dash_pattern,
            dash_offset: 0.0,
        };

        scene.stroke(&stroke, Affine::IDENTITY, border_color, None, &path);
    }
}

fn render_image(
    img: &Image,
    scene: &mut Scene,
    pos: (f64, f64),
    size: Size<f32>,
    radii: (f64, f64, f64, f64),
    fit: &str,
) -> anyhow::Result<()> {
    let width = size.width as f64;
    let height = size.height as f64;

    let rect = RoundedRect::new(pos.0, pos.1, pos.0 + width, pos.1 + height, radii);

    let affine = match fit {
        "fill" => {
            let scale_x = width / img.width as f64;
            let scale_y = height / img.height as f64;

            Affine::scale_non_uniform(scale_x, scale_y)
        }
        "contain" => {
            let scale_x = width / img.width as f64;
            let scale_y = height / img.height as f64;

            let scale = scale_x.min(scale_y);

            Affine::scale_non_uniform(scale, scale)
        }
        "cover" => {
            let scale_x = width / img.width as f64;
            let scale_y = height / img.height as f64;

            let scale = scale_x.max(scale_y);

            Affine::scale_non_uniform(scale, scale)
        }
        "scale-down" => {
            let scale_x = width / img.width as f64;
            let scale_y = height / img.height as f64;

            let scale = scale_x.min(scale_y);
            let scale = scale.min(1.0);

            Affine::scale_non_uniform(scale, scale)
        }
        _ => Affine::IDENTITY,
    };

    let affine = affine.with_translation(pos.into());

    println!("affine: {:?}", affine);
    println!("fit: {:?}", fit);
    println!("width: {:?}", width);
    println!("height: {:?}", height);
    println!("img size: {}x{}", img.width, img.height);
    println!("rect: {:?}", rect);

    scene.fill(Fill::NonZero, Affine::IDENTITY, img, Some(affine), &rect);

    Ok(())
}

#[derive(Debug)]
enum BorderStyle {
    None,
    Hidden,
    Dotted,
    Dashed,
    Solid,
    Double,
    Groove,
    Ridge,
    Inset,
    Outset,
    //DotDash, //TODO: should we support these?
    //DotDotDash,
}

impl BorderStyle {
    fn from_str(style: &str) -> Self {
        match style {
            "none" => Self::None,
            "hidden" => Self::Hidden,
            "dotted" => Self::Dotted,
            "dashed" => Self::Dashed,
            "solid" => Self::Solid,
            "double" => Self::Double,
            "groove" => Self::Groove,
            "ridge" => Self::Ridge,
            "inset" => Self::Inset,
            "outset" => Self::Outset,
            _ => Self::None,
        }
    }
}

//just for debugging
pub fn print_tree(tree: &TaffyTree<GosubId>, root: NodeId, gosub_tree: &RenderTree) {
    println!("TREE");
    print_node(tree, root, false, String::new(), gosub_tree);

    /// Recursive function that prints each node in the tree
    fn print_node(
        tree: &TaffyTree<GosubId>,
        node_id: NodeId,
        has_sibling: bool,
        lines_string: String,
        gosub_tree: &RenderTree,
    ) {
        let layout = &tree.get_final_layout(node_id);
        let display = tree.get_debug_label(node_id);
        let num_children = tree.child_count(node_id);
        let gosub_id = tree.get_node_context(node_id).unwrap();
        let width_style = tree.style(node_id).unwrap().size;

        let fork_string = if has_sibling {
            "├── "
        } else {
            "└── "
        };
        let node = gosub_tree.get_node(*gosub_id).unwrap();
        let mut node_render = String::new();

        match &node.data {
            RenderNodeData::Element(element) => {
                node_render.push('<');
                node_render.push_str(&element.name);
                for (key, value) in element.attributes.iter() {
                    node_render.push_str(&format!(" {}=\"{}\"", key, value));
                }
                node_render.push('>');
            }
            RenderNodeData::Text(text) => {
                let text = text.text.replace('\n', " ");
                node_render.push_str(text.trim());
            }

            _ => {}
        }

        println!(
            "{lines}{fork} {display} [x: {x:<4} y: {y:<4} width: {width:<4} height: {height:<4}] ({key:?}) |{node_render}|{width_style:?}|",
            lines = lines_string,
            fork = fork_string,
            display = display,
            x = layout.location.x,
            y = layout.location.y,
            width = layout.size.width,
            height = layout.size.height,
            key = node_id,
        );
        let bar = if has_sibling { "│   " } else { "    " };
        let new_string = lines_string + bar;

        // Recurse into children
        for (index, child) in tree.child_ids(node_id).enumerate() {
            let has_sibling = index < num_children - 1;
            print_node(tree, child, has_sibling, new_string.clone(), gosub_tree);
        }
    }
}
