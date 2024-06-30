use anyhow::anyhow;
use taffy::{
    AvailableSpace, Layout, NodeId, PrintTree, Size as TSize, TaffyTree, TraversePartialTree,
};
use url::Url;

use gosub_css3::colors::RgbColor;
use gosub_css3::stylesheet::CssValue;
use gosub_html5::node::NodeId as GosubId;
use gosub_render_backend::svg::SvgRenderer;
use gosub_render_backend::{
    Border, BorderSide, BorderStyle, Brush, Color, ImageBuffer, PreRenderText, Rect, RenderBackend,
    RenderBorder, RenderRect, RenderText, Scene as TScene, SizeU32, Text, Transform, FP,
};
use gosub_rendering::layout::generate_taffy_tree;
use gosub_rendering::position::PositionTree;
use gosub_shared::types::Result;
use gosub_styling::render_tree::{RenderNodeData, RenderTree, RenderTreeNode};

use crate::draw::img::request_img;
use crate::render_tree::{load_html_rendertree, NodeID, TreeDrawer};

mod img;

pub trait SceneDrawer<B: RenderBackend> {
    fn draw(&mut self, backend: &mut B, data: &mut B::WindowData<'_>, size: SizeU32);
    fn mouse_move(&mut self, backend: &mut B, data: &mut B::WindowData<'_>, x: FP, y: FP) -> bool;

    fn scroll(&mut self, point: Point);
    fn from_url(url: Url, debug: bool) -> Result<Self>
    where
        Self: Sized;
}

const DEBUG_CONTENT_COLOR: (u8, u8, u8) = (0, 192, 255); //rgb(0, 192, 255)
const DEBUG_PADDING_COLOR: (u8, u8, u8) = (0, 255, 192); //rgb(0, 255, 192)
const DEBUG_BORDER_COLOR: (u8, u8, u8) = (255, 72, 72); //rgb(255, 72, 72)
                                                        // const DEBUG_MARGIN_COLOR: (u8, u8, u8) = (255, 192, 0);

type Point = gosub_shared::types::Point<FP>;

impl<B: RenderBackend> SceneDrawer<B> for TreeDrawer<B> {
    fn draw(&mut self, backend: &mut B, data: &mut B::WindowData<'_>, size: SizeU32) {
        if !self.dirty && self.size == Some(size) {
            return;
        }

        if self.tree_scene.is_none() || self.size != Some(size) {
            self.size = Some(size);

            let mut scene = B::Scene::new(data);

            // Apply new maximums to the scene transform
            if let Some(scene_transform) = self.scene_transform.as_mut() {
                let root_size = self.taffy.get_final_layout(self.root).content_size;
                let max_x = root_size.width - size.width as f32;
                let max_y = root_size.height - size.height as f32;

                let x = scene_transform.tx().min(0.0).max(-max_x);
                let y = scene_transform.ty().min(0.0).max(-max_y);

                scene_transform.set_xy(x, y);
            }

            let mut drawer = Drawer {
                scene: &mut scene,
                drawer: self,
                svg: B::SVGRenderer::new(data),
            };

            drawer.render(size);

            self.tree_scene = Some(scene);

            self.size = Some(size);
        }

        backend.reset(data);

        let bg = Rect::new(0.0, 0.0, size.width as FP, size.height as FP);

        let rect = RenderRect {
            rect: bg,
            transform: None,
            radius: None,
            brush: Brush::color(Color::MAGENTA),
            brush_transform: None,
            border: None,
        };
        //
        backend.draw_rect(data, &rect);

        if let Some(scene) = &self.tree_scene {
            backend.apply_scene(data, scene, self.scene_transform.clone());
        }

        if let Some(scene) = &self.debugger_scene {
            self.dirty = false;
            backend.apply_scene(data, scene, self.scene_transform.clone());
        }
    }

    fn mouse_move(&mut self, _backend: &mut B, data: &mut B::WindowData<'_>, x: FP, y: FP) -> bool {
        let x = x - self
            .scene_transform
            .clone()
            .unwrap_or(B::Transform::IDENTITY)
            .tx();
        let y = y - self
            .scene_transform
            .clone()
            .unwrap_or(B::Transform::IDENTITY)
            .ty();

        if let Some(e) = self.position.find(x, y) {
            if self.last_hover != Some(e) {
                self.last_hover = Some(e);
                if self.debug {
                    let mut scene = B::Scene::new(data);

                    let layout = self.taffy.get_final_layout(e);

                    let content_size = layout.size;
                    let padding = layout.padding;
                    let border_size = layout.border;

                    let Some((x, y)) = self.position.position(e) else {
                        return false;
                    };

                    let content_rect =
                        Rect::new(x, y, content_size.width as FP, content_size.height as FP);

                    let padding_brush =
                        B::Brush::color(B::Color::tuple3(DEBUG_PADDING_COLOR).alpha(127));
                    let content_brush =
                        B::Brush::color(B::Color::tuple3(DEBUG_CONTENT_COLOR).alpha(127));
                    // let margin_brush = B::Brush::color(B::Color::tuple3(DEBUG_MARGIN_COLOR).alpha(127));
                    let border_brush =
                        B::Brush::color(B::Color::tuple3(DEBUG_BORDER_COLOR).alpha(127));

                    let mut border = B::Border::empty();

                    border.left(BorderSide::new(
                        padding.left as FP,
                        BorderStyle::Solid,
                        padding_brush.clone(),
                    ));

                    border.right(BorderSide::new(
                        padding.right as FP,
                        BorderStyle::Solid,
                        padding_brush.clone(),
                    ));

                    border.top(BorderSide::new(
                        padding.top as FP,
                        BorderStyle::Solid,
                        padding_brush.clone(),
                    ));

                    border.bottom(BorderSide::new(
                        padding.bottom as FP,
                        BorderStyle::Solid,
                        padding_brush,
                    ));

                    let padding_border = RenderBorder::new(border);

                    let render_rect = RenderRect {
                        rect: content_rect,
                        transform: None,
                        radius: None,
                        brush: content_brush,
                        brush_transform: None,
                        border: Some(padding_border),
                    };

                    scene.draw_rect(&render_rect);

                    let mut border_border = B::Border::empty();

                    border_border.left(BorderSide::new(
                        border_size.left as FP,
                        BorderStyle::Solid,
                        border_brush.clone(),
                    ));

                    border_border.right(BorderSide::new(
                        border_size.right as FP,
                        BorderStyle::Solid,
                        border_brush.clone(),
                    ));

                    border_border.top(BorderSide::new(
                        border_size.top as FP,
                        BorderStyle::Solid,
                        border_brush.clone(),
                    ));

                    border_border.bottom(BorderSide::new(
                        border_size.bottom as FP,
                        BorderStyle::Solid,
                        border_brush,
                    ));

                    let border_border = RenderBorder::new(border_border);

                    let border_rect = Rect::new(
                        x as FP - border_size.left as FP - padding.left as FP,
                        y as FP - border_size.top as FP - padding.top as FP,
                        (content_size.width + padding.left + padding.right) as FP,
                        (content_size.height + padding.top + padding.bottom) as FP,
                    );

                    let render_rect = RenderRect {
                        rect: border_rect,
                        transform: None,
                        radius: None,
                        brush: Brush::color(Color::TRANSPARENT),
                        brush_transform: None,
                        border: Some(border_border),
                    };

                    scene.draw_rect(&render_rect);

                    self.debugger_scene = Some(scene);
                    self.dirty = true;
                    return true;
                }
            }
            return false;
        };
        false
    }

    fn scroll(&mut self, point: Point) {
        let mut transform = self
            .scene_transform
            .take()
            .unwrap_or(B::Transform::IDENTITY);

        let x = transform.tx() + point.x;
        let y = transform.ty() + point.y;

        let root_size = self.taffy.get_final_layout(self.root).content_size;
        let size = self.size.unwrap_or(SizeU32::ZERO);

        let max_x = root_size.width - size.width as f32;
        let max_y = root_size.height - size.height as f32;

        let x = x.min(0.0).max(-max_x);
        let y = y.min(0.0).max(-max_y);

        transform.set_xy(x, y);

        self.scene_transform = Some(transform);

        self.dirty = true;
    }

    fn from_url(url: Url, debug: bool) -> Result<Self> {
        let mut rt = load_html_rendertree(url.clone())?;

        let (taffy_tree, root) = generate_taffy_tree(&mut rt)?;

        Ok(Self::new(rt, taffy_tree, root, url, debug))
    }
}

struct Drawer<'s, 't, B: RenderBackend> {
    scene: &'s mut B::Scene,
    drawer: &'t mut TreeDrawer<B>,
    svg: B::SVGRenderer,
}

impl<B: RenderBackend> Drawer<'_, '_, B> {
    pub(crate) fn render(&mut self, size: SizeU32) {
        let space = TSize {
            width: AvailableSpace::Definite(size.width as f32),
            height: AvailableSpace::Definite(size.height as f32),
        };

        if let Err(e) = self.drawer.taffy.compute_layout(self.drawer.root, space) {
            eprintln!("Failed to compute layout: {:?}", e);
            return;
        }

        // print_tree(&self.taffy, self.root, &self.style);

        self.drawer.position = PositionTree::from_taffy(&self.drawer.taffy, self.drawer.root);

        self.render_node_with_children(self.drawer.root, Point::ZERO);
    }

    fn render_node_with_children(&mut self, id: NodeID, mut pos: Point) {
        let err = self.render_node(id, &mut pos);
        if let Err(e) = err {
            eprintln!("Error rendering node: {:?}", e);
        }

        let children = match self.drawer.taffy.children(id) {
            Ok(children) => children,
            Err(e) => {
                eprintln!("Error rendering node children: {e}");
                return;
            }
        };

        for child in children {
            self.render_node_with_children(child, pos);
        }
    }

    fn render_node(&mut self, id: NodeID, pos: &mut Point) -> anyhow::Result<()> {
        let gosub_id = *self
            .drawer
            .taffy
            .get_node_context(id)
            .ok_or(anyhow!("Failed to get style id"))?;

        let layout = self.drawer.taffy.get_final_layout(id);

        let node = self
            .drawer
            .style
            .get_node_mut(gosub_id)
            .ok_or(anyhow!("Node not found"))?;

        pos.x += layout.location.x as FP;
        pos.y += layout.location.y as FP;

        let border_radius = render_bg(
            node,
            self.scene,
            layout,
            pos,
            &self.drawer.url,
            &mut self.svg,
        );

        if let RenderNodeData::Element(element) = &node.data {
            if element.name() == "img" {
                let src = element
                    .attributes
                    .get("src")
                    .ok_or(anyhow!("Image element has no src attribute"))?;

                let url =
                    Url::parse(src.as_str()).or_else(|_| self.drawer.url.join(src.as_str()))?;

                let img = request_img(&mut self.svg, &url)?;
                let fit = element
                    .attributes
                    .get("object-fit")
                    .map(|prop| prop.as_str())
                    .unwrap_or("contain");

                println!("Rendering image at: {:?}", pos);
                println!("with size: {:?}", layout.size);

                render_image::<B>(img, self.scene, *pos, layout.size, border_radius, fit)?;
            }
        }

        render_text(node, self.scene, pos, layout);
        Ok(())
    }
}

fn render_text<B: RenderBackend>(
    node: &mut RenderTreeNode<B>,
    scene: &mut B::Scene,
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

        scene.draw_text(&render_text);
    }
}

fn render_bg<B: RenderBackend>(
    node: &mut RenderTreeNode<B>,
    scene: &mut B::Scene,
    layout: &Layout,
    pos: &Point,
    root_url: &Url,
    svg: &mut B::SVGRenderer,
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
            layout.content_size.width as FP,
            layout.content_size.height as FP,
        );

        let rect = RenderRect {
            rect,
            transform: None,
            radius: Some(B::BorderRadius::from(border_radius)),
            brush: Brush::color(bg_color),
            brush_transform: None,
            border,
        };

        scene.draw_rect(&rect);
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

        scene.draw_rect(&rect);
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

        let img = match request_img(svg, &url) {
            Ok(img) => img,
            Err(e) => {
                eprintln!("Error loading image: {:?}", e);
                return border_radius;
            }
        };

        let _ =
            render_image::<B>(img, scene, *pos, layout.size, border_radius, "fill").map_err(|e| {
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
    img: ImageBuffer<B>,
    scene: &mut B::Scene,
    pos: Point,
    size: TSize<f32>,
    radii: (FP, FP, FP, FP),
    fit: &str,
) -> anyhow::Result<()> {
    let width = size.width as FP;
    let height = size.height as FP;

    let rect = Rect::new(pos.x, pos.y, pos.x + width, pos.y + height);

    let img_size = img.size_tuple();

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

    match img {
        ImageBuffer::Image(img) => {
            let rect = RenderRect {
                rect,
                transform: None,
                radius: Some(B::BorderRadius::from(radii)),
                brush: Brush::image(img),
                brush_transform: Some(transform),
                border: None,
            };

            scene.draw_rect(&rect);
        }
        ImageBuffer::Scene(s, _size) => {
            scene.apply_scene(&s, Some(transform)); //TODO we probably want to use a clip layer here
        }
    }

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
    let width = node
        .properties
        .get(&format!("border-{}-width", side.to_str()))
        .map(|prop| {
            prop.compute_value();
            prop.actual.unit_to_px()
        })?;

    let color = node
        .properties
        .get(&format!("border-{}-color", side.to_str()))
        .and_then(|prop| {
            prop.compute_value();

            match &prop.actual {
                CssValue::Color(color) => Some(*color),
                CssValue::String(color) => Some(RgbColor::from(color.as_str())),
                _ => None,
            }
        })?;

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
