use std::io::Read;

use anyhow::anyhow;
use image::DynamicImage;
use taffy::{
    AvailableSpace, Layout, NodeId, PrintTree, Size as TSize, TaffyTree, TraversePartialTree,
};
use url::Url;

use gosub_html5::node::NodeId as GosubId;
use gosub_render_backend::{
    Border, BorderSide, BorderStyle, Brush, Color, Image, PreRenderText, Rect, RenderBackend,
    RenderBorder, RenderRect, RenderText, SizeU32, Text, Transform, FP,
};
use gosub_rendering::layout::generate_taffy_tree;
use gosub_rendering::position::PositionTree;
use gosub_shared::types::Result;
use gosub_styling::css_colors::RgbColor;
use gosub_styling::css_values::CssValue;
use gosub_styling::render_tree::{RenderNodeData, RenderTree, RenderTreeNode};

use crate::render_tree::{load_html_rendertree, NodeID, TreeDrawer};

pub trait SceneDrawer<B: RenderBackend> {
    fn draw(&mut self, backend: &mut B, data: &mut B::WindowData<'_>, size: SizeU32);
    fn mouse_move(&mut self, backend: &mut B, data: &mut B::WindowData<'_>, x: f64, y: f64);

    fn from_url(url: Url) -> Result<Self>
    where
        Self: Sized;
}

type Point = gosub_shared::types::Point<FP>;

impl<B: RenderBackend> SceneDrawer<B> for TreeDrawer<B> {
    fn draw(&mut self, backend: &mut B, data: &mut B::WindowData<'_>, size: SizeU32) {
        if self.size == Some(size) {
            //This check needs to be updated in the future, when the tree is mutable
            return;
        }

        self.size = Some(size);

        backend.reset(data);
        self.render(backend, data, size);
    }

    fn mouse_move(&mut self, _backend: &mut B, _data: &mut B::WindowData<'_>, x: f64, y: f64) {
        if let Some(e) = self.position.find(x as f32, y as f32) {
            if self.last_hover != Some(e) {
                self.last_hover = Some(e);
                let Some(node_id) = self.taffy.get_node_context(e) else {
                    return;
                };

                let Some(node) = self.style.get_node(*node_id) else {
                    return;
                };

                println!("Hovering over: {:?} ({e:?})@({x},{y})", node.data);
            }
        };
    }

    fn from_url(url: Url) -> Result<Self> {
        let mut rt = load_html_rendertree(url.clone())?;

        let (taffy_tree, root) = generate_taffy_tree(&mut rt)?;

        Ok(Self::new(rt, taffy_tree, root, url))
    }
}

impl<B: RenderBackend> TreeDrawer<B> {
    pub(crate) fn render(&mut self, backend: &mut B, data: &mut B::WindowData<'_>, size: SizeU32) {
        let space = TSize {
            width: AvailableSpace::Definite(size.width as f32),
            height: AvailableSpace::Definite(size.height as f32),
        };

        if let Err(e) = self.taffy.compute_layout(self.root, space) {
            eprintln!("Failed to compute layout: {:?}", e);
            return;
        }

        // print_tree(&self.taffy, self.root, &self.style);

        self.position = PositionTree::from_taffy(&self.taffy, self.root);

        let bg = Rect::new(0.0, 0.0, size.width as FP, size.height as FP);

        let rect = RenderRect {
            rect: bg,
            transform: None,
            radius: None,
            brush: Brush::color(Color::BLACK),
            brush_transform: None,
            border: None,
        };
        //
        backend.draw_rect(data, &rect);

        self.render_node_with_children(self.root, backend, data, Point::ZERO);
    }

    fn render_node_with_children(
        &mut self,
        id: NodeID,
        backend: &mut B,
        data: &mut B::WindowData<'_>,
        mut pos: Point,
    ) {
        let err = self.render_node(id, backend, data, &mut pos);
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
            self.render_node_with_children(child, backend, data, pos);
        }
    }

    fn render_node(
        &mut self,
        id: NodeID,
        backend: &mut B,
        data: &mut B::WindowData<'_>,
        pos: &mut Point,
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

        pos.x += layout.location.x as FP;
        pos.y += layout.location.y as FP;

        let border_radius = render_bg(node, backend, data, layout, pos, &self.url);

        if let RenderNodeData::Element(element) = &node.data {
            if element.name() == "img" {
                let src = element
                    .attributes
                    .get("src")
                    .ok_or(anyhow!("Image element has no src attribute"))?;

                let url = Url::parse(src.as_str()).or_else(|_| self.url.join(src.as_str()))?;

                let img = if url.scheme() == "file" {
                    let path = url.as_str().trim_start_matches("file://");

                    image::open(path)?
                } else {
                    let res = gosub_net::http::ureq::get(url.as_str()).call()?;

                    let mut img = Vec::with_capacity(
                        res.header("Content-Length")
                            .unwrap_or("1024")
                            .parse::<usize>()?,
                    );

                    res.into_reader().read_to_end(&mut img)?;

                    image::load_from_memory(&img)?
                };

                let fit = element
                    .attributes
                    .get("object-fit")
                    .map(|prop| prop.as_str())
                    .unwrap_or("contain");

                println!("Rendering image at: {:?}", pos);
                println!("with size: {:?}", layout.size);

                render_image(img, backend, data, *pos, layout.size, border_radius, fit)?;
            }
        }

        render_text(node, backend, data, pos, layout);
        Ok(())
    }
}

fn render_text<B: RenderBackend>(
    node: &mut RenderTreeNode<B>,
    backend: &mut B,
    data: &mut B::WindowData<'_>,
    pos: &Point,
    layout: &Layout,
) {
    if let RenderNodeData::Text(text) = &mut node.data {
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
            .map(|color| Color::rgba(color.r as u8, color.g as u8, color.b as u8, color.a as u8))
            .unwrap_or(Color::BLACK);

        let text = Text::new(&mut text.prerender);

        let rect = Rect::new(
            pos.x as FP,
            pos.y as FP,
            layout.size.width as FP,
            layout.size.height as FP,
        );

        let render_text = RenderText {
            text,
            rect,
            transform: None,
            brush: Brush::color(color),
            brush_transform: None,
        };

        backend.draw_text(data, &render_text);
    }
}

fn render_bg<B: RenderBackend>(
    node: &mut RenderTreeNode<B>,
    backend: &mut B,
    data: &mut B::WindowData<'_>,
    layout: &Layout,
    pos: &Point,
    root_url: &Url,
) -> (FP, FP, FP, FP) {
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
        .map(|color| Color::rgba(color.r as u8, color.g as u8, color.b as u8, color.a as u8));

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
        border_radius_top as FP,
        border_radius_right as FP,
        border_radius_bottom as FP,
        border_radius_left as FP,
    );

    let border = get_border(node).map(|border| RenderBorder::new(border));

    if let Some(bg_color) = bg_color {
        let rect = Rect::new(
            pos.x as FP,
            pos.y as FP,
            layout.size.width as FP,
            layout.size.height as FP,
        );

        let rect = RenderRect {
            rect,
            transform: None,
            radius: Some(B::BorderRadius::from(border_radius)),
            brush: Brush::color(bg_color),
            brush_transform: None,
            border,
        };

        backend.draw_rect(data, &rect);
    } else if let Some(border) = border {
        let rect = Rect::new(
            pos.x as FP,
            pos.y as FP,
            layout.size.width as FP,
            layout.size.height as FP,
        );

        let rect = RenderRect {
            rect,
            transform: None,
            radius: Some(B::BorderRadius::from(border_radius)),
            brush: Brush::color(Color::TRANSPARENT),
            brush_transform: None,
            border: Some(border),
        };

        backend.draw_rect(data, &rect);
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

        let Ok(res) = gosub_net::http::ureq::get(url.as_str()).call() else {
            return border_radius;
        };

        let mut img = Vec::with_capacity(
            res.header("Content-Length")
                .unwrap_or("1024")
                .parse::<usize>()
                .unwrap_or(1024),
        );

        let _ = res.into_reader().read_to_end(&mut img); //TODO: handle error

        let Ok(img) = image::load_from_memory(&img) else {
            return border_radius;
        };

        let _ = render_image(img, backend, data, *pos, layout.size, border_radius, "fill").map_err(
            |e| {
                eprintln!("Error rendering image: {:?}", e);
            },
        );
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

fn render_image<B: RenderBackend>(
    img: DynamicImage,
    backend: &mut B,
    data: &mut B::WindowData<'_>,
    pos: Point,
    size: TSize<f32>,
    radii: (FP, FP, FP, FP),
    fit: &str,
) -> anyhow::Result<()> {
    let width = size.width as FP;
    let height = size.height as FP;

    let rect = Rect::new(pos.x, pos.y, pos.x + width, pos.y + height);

    let img_size = (img.width() as FP, img.height() as FP);

    let transform = match fit {
        "fill" => {
            let scale_x = width / img_size.0;
            let scale_y = height / img_size.1;

            B::Transform::scale_xy(scale_x, scale_y)
        }
        "contain" => {
            let scale_x = width / img_size.0;
            let scale_y = height / img_size.1;

            let scale = scale_x.min(scale_y);

            Transform::scale(scale)
        }
        "cover" => {
            let scale_x = width / img_size.0;
            let scale_y = height / img_size.1;

            let scale = scale_x.max(scale_y);

            Transform::scale(scale)
        }
        "scale-down" => {
            let scale_x = width / img_size.0;
            let scale_y = height / img_size.1;

            let scale = scale_x.min(scale_y);
            let scale = scale.min(1.0);

            Transform::scale(scale)
        }
        _ => Transform::IDENTITY,
    };

    let transform = transform.with_translation(pos);

    let rect = RenderRect {
        rect,
        transform: None,
        radius: Some(B::BorderRadius::from(radii)),
        brush: Brush::image(Image::new(img_size, img.into_rgba8().into_raw())),
        brush_transform: Some(transform),
        border: None,
    };

    backend.draw_rect(data, &rect);

    Ok(())
}

//just for debugging
pub fn print_tree<B: RenderBackend>(
    tree: &TaffyTree<GosubId>,
    root: NodeId,
    gosub_tree: &RenderTree<B>,
) {
    println!("TREE");
    print_node(tree, root, false, String::new(), gosub_tree);

    /// Recursive function that prints each node in the tree
    fn print_node<B: RenderBackend>(
        tree: &TaffyTree<GosubId>,
        node_id: NodeId,
        has_sibling: bool,
        lines_string: String,
        gosub_tree: &RenderTree<B>,
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
                let text = text.prerender.value().replace('\n', " ");
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

fn get_border<B: RenderBackend>(node: &mut RenderTreeNode<B>) -> Option<B::Border> {
    let left = get_border_side(node, Side::Left);
    let right = get_border_side(node, Side::Right);
    let top = get_border_side(node, Side::Top);
    let bottom = get_border_side(node, Side::Bottom);

    if left.is_none() && right.is_none() && top.is_none() && bottom.is_none() {
        return None;
    }

    let mut border = B::Border::empty();

    if let Some(left) = left {
        border.left(left)
    }

    if let Some(right) = right {
        border.right(right)
    }

    if let Some(top) = top {
        border.top(top)
    }

    if let Some(bottom) = bottom {
        border.bottom(bottom)
    }

    Some(border)
}

fn get_border_side<B: RenderBackend>(
    node: &mut RenderTreeNode<B>,
    side: Side,
) -> Option<B::BorderSide> {
    let Some(width) = node
        .properties
        .get(&format!("border-{}-width", side.to_str()))
        .map(|prop| {
            prop.compute_value();
            prop.actual.unit_to_px()
        })
    else {
        return None;
    };

    let Some(color) = node
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
    else {
        return None;
    };

    let style = node
        .properties
        .get(&format!("border-{}-style", side.to_str()))
        .map(|prop| {
            prop.compute_value();
            prop.actual.to_string()
        })
        .unwrap_or("none".to_string());

    let style = BorderStyle::from_str(&style);

    let brush = Brush::color(Color::rgba(
        color.r as u8,
        color.g as u8,
        color.b as u8,
        color.a as u8,
    ));

    Some(BorderSide::new(width as FP, style, brush))
}
