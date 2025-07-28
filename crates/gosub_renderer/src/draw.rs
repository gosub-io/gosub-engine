use crate::debug::scale::px_scale;
use crate::draw::img::request_img;
use crate::draw::img_cache::ImageCache;
use crate::draw::testing::{test_add_element, test_restyle_element};
use crate::render_tree::{load_html_rendertree, load_html_rendertree_fetcher, load_html_rendertree_source};
use anyhow::anyhow;
use gosub_interface::config::{HasDocument, HasDrawComponents, HasHtmlParser};
use gosub_interface::css3::{CssProperty, CssPropertyMap, CssValue};

use gosub_interface::draw::TreeDrawer;
use gosub_interface::eventloop::EventLoopHandle;
use gosub_interface::layout::{Layout, LayoutTree, Layouter};
use gosub_interface::render_backend::{
    Border, BorderSide, BorderStyle, Brush, Color, ImageBuffer, ImgCache, NodeDesc, Rect, RenderBackend, RenderBorder,
    RenderRect, RenderText, Scene as TScene, Text, Transform,
};
use gosub_interface::render_tree;
use gosub_interface::render_tree::RenderTreeNode as _;
use gosub_interface::svg::SvgRenderer;
use gosub_net::http::fetcher::Fetcher;
use gosub_rendering::position::PositionTree;
use gosub_rendering::render_tree::RenderTree;
use gosub_shared::geo::{Size, SizeU32, FP};
use gosub_shared::node::NodeId;
use gosub_shared::types::Result;
use log::{error, info};
use std::future::Future;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use url::Url;

mod img;
pub mod img_cache;
mod testing;

const DEBUG_CONTENT_COLOR: (u8, u8, u8) = (0, 192, 255);
const DEBUG_PADDING_COLOR: (u8, u8, u8) = (0, 255, 192);
const DEBUG_BORDER_COLOR: (u8, u8, u8) = (255, 72, 72);

type Point = gosub_shared::types::Point<FP>;

#[derive(Debug)]
pub struct TreeDrawerImpl<C: HasDrawComponents> {
    pub(crate) tree: C::RenderTree,
    pub(crate) fetcher: Arc<Fetcher>,
    pub(crate) layouter: C::Layouter,
    pub(crate) size: Option<SizeU32>,
    pub(crate) position: PositionTree<C>,
    pub(crate) last_hover: Option<NodeId>,
    pub(crate) debug: bool,
    pub(crate) dirty: bool,
    pub(crate) debugger_scene: Option<<C::RenderBackend as RenderBackend>::Scene>,
    pub(crate) tree_scene: Option<<C::RenderBackend as RenderBackend>::Scene>,
    pub(crate) selected_element: Option<NodeId>,
    pub(crate) scene_transform: Option<<C::RenderBackend as RenderBackend>::Transform>,
    pub(crate) img_cache: ImageCache<C::RenderBackend>,
}

impl<C: HasDrawComponents> TreeDrawerImpl<C> {
    pub fn new(tree: C::RenderTree, layouter: C::Layouter, fetcher: Arc<Fetcher>, debug: bool) -> Self {
        Self {
            tree,
            fetcher,
            layouter,
            size: None,
            position: PositionTree::default(),
            last_hover: None,
            debug,
            debugger_scene: None,
            dirty: false,
            tree_scene: None,
            selected_element: None,
            scene_transform: None,
            img_cache: ImageCache::new(),
        }
    }
}

impl<C: HasDrawComponents<RenderTree = RenderTree<C>, LayoutTree = RenderTree<C>> + HasHtmlParser + HasDocument>
    TreeDrawer<C> for TreeDrawerImpl<C>
{
    type ImgCache = ImageCache<C::RenderBackend>;

    fn draw(&mut self, size: SizeU32, el: &impl EventLoopHandle<C>) -> <C::RenderBackend as RenderBackend>::Scene {
        if self.tree_scene.is_none() || self.size != Some(size) || !self.dirty {
            self.size = Some(size);

            let mut scene = <C::RenderBackend as RenderBackend>::Scene::new();

            // Apply new maximums to the scene transform
            if let Some(scene_transform) = self.scene_transform.as_mut() {
                let root_size = self.tree.get_root().layout.content();
                // Calculate max_x and max_y, ensuring they are not negative cause if the root size is smaller than the size, max_x/max_y should be 0
                let max_x = (root_size.width - size.width as f32).max(0.0);
                let max_y = (root_size.height - size.height as f32).max(0.0);

                let x = scene_transform.tx().min(0.0).max(-max_x);
                let y = scene_transform.ty().min(0.0).max(-max_y);

                scene_transform.set_xy(x, y);
            }

            let mut drawer = Drawer {
                scene: &mut scene,
                drawer: self,
                svg: Arc::new(Mutex::new(<C::RenderBackend as RenderBackend>::SVGRenderer::new())),
                el,
            };

            drawer.render(size);

            self.tree_scene = Some(scene);

            self.size = Some(size);
        }

        let bg = Rect::new(0.0, 0.0, size.width as FP, size.height as FP);

        let rect = RenderRect {
            rect: bg,
            transform: None,
            radius: None,
            brush: Brush::color(Color::WHITE),
            brush_transform: None,
            border: None,
        };

        let mut root_scene = <C::RenderBackend as RenderBackend>::Scene::new();

        root_scene.draw_rect(&rect);

        if let Some(scene) = &self.tree_scene {
            root_scene.apply_scene(scene, self.scene_transform.clone());
        } else {
            println!("No scene");
        }

        if self.dirty {
            if let Some(id) = self.selected_element {
                self.debug_annotate(id);
            }
        }

        if let Some(scene) = &self.debugger_scene {
            self.dirty = false;
            root_scene.apply_scene(scene, self.scene_transform.clone());
        }

        if self.debug {
            let pos = self
                .scene_transform
                .as_ref()
                .map_or(Point::ZERO, |x| Point::new(x.tx(), x.ty()));

            let scale = px_scale::<C::RenderBackend>(size, pos, self.size.as_ref().map_or(0.0, |x| x.width as f32));

            root_scene.apply_scene(&scale, None);
        }

        if self.dirty {
            self.dirty = false;
            // el.redraw();
        }

        root_scene
    }

    fn mouse_move(&mut self, x: FP, y: FP) -> bool {
        let x = x - self.scene_transform.clone().unwrap_or(Transform::IDENTITY).tx();
        let y = y - self.scene_transform.clone().unwrap_or(Transform::IDENTITY).ty();

        if let Some(e) = self.position.find(x, y) {
            if self.last_hover != Some(e) {
                self.last_hover = Some(e);
                if self.debug {
                    return self.debug_annotate(e);
                }
            }
            return false;
        }
        false
    }

    fn scroll(&mut self, point: Point) {
        let mut transform = self.scene_transform.take().unwrap_or(Transform::IDENTITY);

        let x = transform.tx() + point.x;
        let y = transform.ty() + point.y;

        let root_size = self.tree.get_root().layout.content();
        let size = self.size.unwrap_or(SizeU32::ZERO);

        let max_x = root_size.width - size.width as f32;
        let max_y = root_size.height - size.height as f32;

        let x = x.min(0.0).max(-max_x);
        let y = y.min(0.0).max(-max_y);

        transform.set_xy(x, y);

        self.scene_transform = Some(transform);

        self.dirty = true;
    }

    async fn from_url(url: Url, layouter: C::Layouter, debug: bool) -> Result<(Self, C::Document)> {
        let (rt, handle, fetcher) = load_html_rendertree::<C>(url.clone(), None).await?;

        Ok((Self::new(rt, layouter, Arc::new(fetcher), debug), handle))
    }

    fn from_source(url: Url, source_html: &str, layouter: C::Layouter, debug: bool) -> Result<(Self, C::Document)> {
        let fetcher = Fetcher::new(url.clone());
        let (rt, handle) = load_html_rendertree_source::<C>(url, source_html)?;

        Ok((Self::new(rt, layouter, Arc::new(fetcher), debug), handle))
    }

    async fn with_fetcher(
        url: Url,
        fetcher: Arc<Fetcher>,
        layouter: C::Layouter,
        debug: bool,
    ) -> Result<(Self, C::Document)> {
        let (rt, handle) = load_html_rendertree_fetcher::<C>(url.clone(), &fetcher).await?;

        Ok((Self::new(rt, layouter, fetcher, debug), handle))
    }

    fn clear_buffers(&mut self) {
        self.tree_scene = None;
        self.debugger_scene = None;
        self.last_hover = None;
        self.dirty = true;
    }

    fn toggle_debug(&mut self) {
        self.debug = !self.debug;
        self.dirty = true;
        self.last_hover = None;
        self.debugger_scene = None;
    }

    fn select_element(&mut self, id: NodeId) {
        if id.as_usize() == u64::MAX as usize {
            test_add_element(self);
            return;
        }

        if id.as_usize() == u64::MAX as usize - 1 {
            test_restyle_element(self);
            return;
        }

        self.selected_element = Some(id);
        self.dirty = true;
    }

    fn unselect_element(&mut self) {
        self.selected_element = None;
        self.debugger_scene = None;
        self.dirty = true;
    }

    fn info(&mut self, id: NodeId, sender: Sender<NodeDesc>) {
        let _ = sender.send(self.tree.desc_node(id));
    }

    fn send_nodes(&mut self, sender: Sender<NodeDesc>) {
        let _ = sender.send(self.tree.desc());
    }

    fn set_needs_redraw(&mut self) {
        self.dirty = true;
    }

    fn get_img_cache(&mut self) -> &mut Self::ImgCache {
        &mut self.img_cache
    }

    fn make_dirty(&mut self) {
        self.dirty = true;
    }

    fn delete_scene(&mut self) {
        self.tree_scene = None;
        self.debugger_scene = None;
    }

    fn reload(&mut self, el: impl EventLoopHandle<C>) -> impl Future<Output = Result<C::Document>> + 'static {
        let fetcher = self.fetcher.clone();

        async move {
            info!("Reloading tab");

            let (rt, handle) = match load_html_rendertree_fetcher::<C>(fetcher.base().clone(), &fetcher).await {
                Ok(rt) => rt,
                Err(e) => {
                    error!("Failed to reload tab: {e}");
                    return Err(e);
                }
            };

            el.reload_from(rt);

            Ok(handle)
        }
    }

    fn navigate(
        &mut self,
        url: Url,
        el: impl EventLoopHandle<C>,
    ) -> impl Future<Output = Result<C::Document>> + 'static {
        let fetcher = self.fetcher.clone();

        async move {
            info!("Navigating to {url}");

            let (rt, handle) = match load_html_rendertree_fetcher::<C>(url.clone(), &fetcher).await {
                Ok(rt) => rt,
                Err(e) => {
                    error!("Failed to navigate to {url}: {e}");
                    return Err(e);
                }
            };

            el.reload_from(rt);

            Ok(handle)
        }
    }

    fn reload_from(&mut self, tree: C::RenderTree) {
        self.tree = tree;
        self.size = None;
        self.position = PositionTree::default();
        self.last_hover = None;
        self.debugger_scene = None;
        self.dirty = false;
        self.tree_scene = None;
        self.selected_element = None;
        self.scene_transform = None;
    }
}

struct Drawer<'s, 't, C: HasDrawComponents, EL: EventLoopHandle<C>> {
    scene: &'s mut <C::RenderBackend as RenderBackend>::Scene,
    drawer: &'t mut TreeDrawerImpl<C>,
    svg: Arc<Mutex<<C::RenderBackend as RenderBackend>::SVGRenderer>>,
    el: &'t EL,
}

impl<
        C: HasDrawComponents<LayoutTree = RenderTree<C>, RenderTree = RenderTree<C>> + HasHtmlParser,
        EL: EventLoopHandle<C>,
    > Drawer<'_, '_, C, EL>
{
    pub(crate) fn render(&mut self, size: SizeU32) {
        let root = self.drawer.tree.root();
        if let Err(e) = self.drawer.layouter.layout(&mut self.drawer.tree, root, size) {
            eprintln!("Failed to compute layout: {e:?}");
            return;
        }

        self.drawer.position = PositionTree::<C>::from_tree(&self.drawer.tree);

        self.render_node_with_children(self.drawer.tree.root(), Point::ZERO);
    }

    fn render_node_with_children(&mut self, id: NodeId, mut pos: Point) {
        let err = self.render_node(id, &mut pos);
        if let Err(e) = err {
            eprintln!("Error rendering node: {e}");
        }

        let Some(children) = self.drawer.tree.children(id) else {
            eprintln!("Error rendering node children");
            return;
        };

        for child in children {
            self.render_node_with_children(child, pos);
        }
    }

    fn render_node(&mut self, id: NodeId, pos: &mut Point) -> Result<()> {
        let node = self.drawer.tree.get_node(id).ok_or(anyhow!("Node {id} not found"))?;

        let p = node.layout().rel_pos();
        pos.x += p.x as FP;
        pos.y += p.y as FP;

        let (border_radius, new_size) = render_bg::<C>(
            node,
            self.scene,
            pos,
            self.svg.clone(),
            self.drawer.fetcher.clone(),
            &mut self.drawer.img_cache,
            self.el,
        );

        let mut size_change = new_size;

        if node.name() == "img" {
            if let Some(attributes) = node.element_attributes() {
                let url: &str = attributes
                    .get("src")
                    .ok_or(anyhow!("Image element has no src attribute"))?;

                let size = node.layout().size_or().map(|x| x.u32());

                let img = request_img::<C>(
                    self.drawer.fetcher.clone(),
                    self.svg.clone(),
                    url,
                    size,
                    &mut self.drawer.img_cache,
                    self.el,
                )?;

                if size.is_none() {
                    size_change = Some(img.size());
                }

                let fit = node
                    .props()
                    .get("object-fit")
                    .and_then(|prop| prop.as_string())
                    .unwrap_or("contain");

                let size = size.unwrap_or(img.size()).f32();

                render_image::<C::RenderBackend>(img, *pos, size, border_radius, fit, self.scene)?;
            }
        }

        render_text::<C>(node, pos, self.scene);

        if let Some(new) = size_change {
            let node = self
                .drawer
                .tree
                .get_node_mut(id)
                .ok_or(anyhow!("Node {id} not found"))?;

            node.layout_mut().set_size(new);

            self.drawer.set_needs_redraw();
        }

        Ok(())
    }
}

fn render_text<C: HasDrawComponents>(
    node: &<C::RenderTree as render_tree::RenderTree<C>>::Node,
    pos: &Point,
    scene: &mut <C::RenderBackend as RenderBackend>::Scene,
) {
    let color = node
        .props()
        .get("color")
        .and_then(gosub_interface::css3::CssProperty::parse_color)
        .map_or(Color::BLACK, |color| {
            Color::rgba(color.0 as u8, color.1 as u8, color.2 as u8, color.3 as u8)
        });

    if let Some((_, layout)) = &node.text_data() {
        let text = layout
            .iter()
            .map(|layout| {
                let text: <C::RenderBackend as RenderBackend>::Text = Text::new(layout);
                text
            })
            .collect::<Vec<_>>();

        let size = node.layout().size();

        let rect = Rect::new(pos.x as FP, pos.y as FP, size.width as FP, size.height as FP);

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

fn render_image<B: RenderBackend>(
    img: ImageBuffer<B>,
    pos: Point,
    size: Size,
    radii: (FP, FP, FP, FP),
    fit: &str,
    scene: &mut B::Scene,
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

/*
//just for debugging
pub fn print_tree<B: RenderBackend, L: Layouter>(
    tree: &TaffyTree<GosubId>,
    root: NodeId,
    gosub_tree: &RenderTree<B>,
) {
    println!("TREE");
    print_node(tree, root, false, String::new(), gosub_tree);

    /// Recursive function that prints each node in the tree
    fn print_node<B: RenderBackend, L: Layouter>(
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
*/

fn render_bg<C: HasDrawComponents>(
    node: &<C::RenderTree as render_tree::RenderTree<C>>::Node,
    scene: &mut <C::RenderBackend as RenderBackend>::Scene,
    pos: &Point,
    svg: Arc<Mutex<<C::RenderBackend as RenderBackend>::SVGRenderer>>,
    fetcher: Arc<Fetcher>,
    img_cache: &mut ImageCache<C::RenderBackend>,
    el: &impl EventLoopHandle<C>,
) -> ((FP, FP, FP, FP), Option<SizeU32>) {
    let bg_color = node
        .props()
        .get("background-color")
        .and_then(gosub_interface::css3::CssProperty::parse_color)
        .map(|color| Color::rgba(color.0 as u8, color.1 as u8, color.2 as u8, color.3 as u8));

    let border_radius_left = node
        .props()
        .get("border-radius-left")
        .map_or(0.0, |prop| f64::from(prop.unit_to_px()));

    let border_radius_right = node
        .props()
        .get("border-radius-right")
        .map_or(0.0, |prop| f64::from(prop.unit_to_px()));

    let border_radius_top = node
        .props()
        .get("border-radius-top")
        .map_or(0.0, |prop| f64::from(prop.unit_to_px()));

    let border_radius_bottom = node
        .props()
        .get("border-radius-bottom")
        .map_or(0.0, |prop| f64::from(prop.unit_to_px()));

    let border_radius = (
        border_radius_top as FP,
        border_radius_right as FP,
        border_radius_bottom as FP,
        border_radius_left as FP,
    );

    let border = get_border::<C>(node).map(RenderBorder::new);

    if let Some(bg_color) = bg_color {
        let size = node.layout().size();

        let rect = Rect::new(pos.x as FP, pos.y as FP, size.width as FP, size.height as FP);

        let rect = RenderRect {
            rect,
            transform: None,
            radius: Some(<C::RenderBackend as RenderBackend>::BorderRadius::from(border_radius)),
            brush: Brush::color(bg_color),
            brush_transform: None,
            border,
        };

        scene.draw_rect(&rect);
    } else if let Some(border) = border {
        let size = node.layout().size();

        let rect = Rect::new(pos.x as FP, pos.y as FP, size.width as FP, size.height as FP);

        let rect = RenderRect {
            rect,
            transform: None,
            radius: Some(<C::RenderBackend as RenderBackend>::BorderRadius::from(border_radius)),
            brush: Brush::color(Color::TRANSPARENT),
            brush_transform: None,
            border: Some(border),
        };

        scene.draw_rect(&rect);
    }

    let background_image = node.props().get("background-image").and_then(|prop| prop.as_function());

    let mut img_size = None;

    #[allow(clippy::collapsible_match)]
    if let Some((function, args)) = background_image {
        #[allow(clippy::single_match)]
        match function {
            "url" => {
                if let Some(url) = args.first().and_then(|url| url.as_string()) {
                    let size = node.layout().size_or().map(|x| x.u32());

                    let img = match request_img::<C>(fetcher.clone(), svg.clone(), url, size, img_cache, el) {
                        Ok(img) => img,
                        Err(e) => {
                            eprintln!("Error loading image: {e:?}");
                            return (border_radius, None);
                        }
                    };

                    if size.is_none() {
                        img_size = Some(img.size());
                    }

                    let _ =
                        render_image::<C::RenderBackend>(img, *pos, node.layout().size(), border_radius, "fill", scene)
                            .map_err(|e| {
                                eprintln!("Error rendering image: {e:?}");
                            });
                }
            }

            _ => {}
        }
    }

    (border_radius, img_size)
}

fn get_border<C: HasDrawComponents>(
    node: &<C::RenderTree as render_tree::RenderTree<C>>::Node,
) -> Option<<C::RenderBackend as RenderBackend>::Border> {
    let left = get_border_side::<C>(node, Side::Left);
    let right = get_border_side::<C>(node, Side::Right);
    let top = get_border_side::<C>(node, Side::Top);
    let bottom = get_border_side::<C>(node, Side::Bottom);

    if left.is_none() && right.is_none() && top.is_none() && bottom.is_none() {
        return None;
    }

    let mut border = <C::RenderBackend as RenderBackend>::Border::empty();

    if let Some(left) = left {
        border.left(left);
    }

    if let Some(right) = right {
        border.right(right);
    }

    if let Some(top) = top {
        border.top(top);
    }

    if let Some(bottom) = bottom {
        border.bottom(bottom);
    }

    Some(border)
}

fn get_border_side<C: HasDrawComponents>(
    node: &<C::RenderTree as render_tree::RenderTree<C>>::Node,
    side: Side,
) -> Option<<C::RenderBackend as RenderBackend>::BorderSide> {
    let width = node
        .props()
        .get(&format!("border-{}-width", side.to_str()))
        .map(gosub_interface::css3::CssProperty::unit_to_px)?;

    let color = node
        .props()
        .get(&format!("border-{}-color", side.to_str()))
        .and_then(gosub_interface::css3::CssProperty::parse_color)?;

    let style = node
        .props()
        .get(&format!("border-{}-style", side.to_str()))
        .and_then(|prop| prop.as_string())
        .unwrap_or("none");

    let style = BorderStyle::from_str(style);

    let brush = Brush::color(Color::rgba(color.0 as u8, color.1 as u8, color.2 as u8, color.3 as u8));

    Some(BorderSide::new(width as FP, style, brush))
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

impl<C: HasDrawComponents<RenderTree = RenderTree<C>, LayoutTree = RenderTree<C>>> TreeDrawerImpl<C> {
    fn debug_annotate(&mut self, e: NodeId) -> bool {
        let Some(node) = self.tree.get_node(e) else {
            return false;
        };

        let mut scene = <C::RenderBackend as RenderBackend>::Scene::new();

        let Some(layout) = self.tree.get_layout(e) else {
            return false;
        };
        let size = layout.size();

        let padding = layout.padding();
        let border_size = layout.border();

        let Some((x, y)) = self.position.position(e) else {
            return false;
        };

        println!("Annotating: {node:?}");
        println!("At: {:?} size: {size:?}", (x, y));

        let content_rect = Rect::new(x, y, size.width as FP, size.height as FP);

        let padding_brush = <C::RenderBackend as RenderBackend>::Brush::color(
            <C::RenderBackend as RenderBackend>::Color::tuple3(DEBUG_PADDING_COLOR).alpha(127),
        );
        let content_brush = <C::RenderBackend as RenderBackend>::Brush::color(
            <C::RenderBackend as RenderBackend>::Color::tuple3(DEBUG_CONTENT_COLOR).alpha(127),
        );
        // let margin_brush = <C::RenderBackend as RenderBackend>::Brush::color(<C::RenderBackend as RenderBackend>::Color::tuple3(DEBUG_MARGIN_COLOR).alpha(127));
        let border_brush = <C::RenderBackend as RenderBackend>::Brush::color(
            <C::RenderBackend as RenderBackend>::Color::tuple3(DEBUG_BORDER_COLOR).alpha(127),
        );

        let mut border = <C::RenderBackend as RenderBackend>::Border::empty();

        border.left(BorderSide::new(
            padding.x2 as FP,
            BorderStyle::Solid,
            padding_brush.clone(),
        ));

        border.right(BorderSide::new(
            padding.x1 as FP,
            BorderStyle::Solid,
            padding_brush.clone(),
        ));

        border.top(BorderSide::new(
            padding.y1 as FP,
            BorderStyle::Solid,
            padding_brush.clone(),
        ));

        border.bottom(BorderSide::new(padding.y2 as FP, BorderStyle::Solid, padding_brush));

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

        let mut border_border = <C::RenderBackend as RenderBackend>::Border::empty();

        border_border.left(BorderSide::new(
            border_size.x2 as FP,
            BorderStyle::Solid,
            border_brush.clone(),
        ));

        border_border.right(BorderSide::new(
            border_size.x1 as FP,
            BorderStyle::Solid,
            border_brush.clone(),
        ));

        border_border.top(BorderSide::new(
            border_size.y1 as FP,
            BorderStyle::Solid,
            border_brush.clone(),
        ));

        border_border.bottom(BorderSide::new(border_size.y2 as FP, BorderStyle::Solid, border_brush));

        let border_border = RenderBorder::new(border_border);

        let border_rect = Rect::new(
            x as FP - border_size.x2 as FP - padding.x2 as FP,
            y as FP - border_size.y1 as FP - padding.y1 as FP,
            (size.width + padding.x2 + padding.x1) as FP,
            (size.height + padding.y1 + padding.y2) as FP,
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

        true
    }
}
