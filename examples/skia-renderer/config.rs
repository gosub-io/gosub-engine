use futures::channel::mpsc::UnboundedSender;
use log::info;
use gosub_cairo::{CairoBackend, Scene};
use gosub_css3::system::Css3System;
use gosub_fontmanager::FontManager;
use gosub_html5::document::builder::DocumentBuilderImpl;
use gosub_html5::document::document_impl::DocumentImpl;
use gosub_html5::document::fragment::DocumentFragmentImpl;
use gosub_html5::parser::Html5Parser;
use gosub_interface::chrome::ChromeHandle;
use gosub_interface::config::{HasChrome, HasCssSystem, HasDocument, HasHtmlParser, HasLayouter, HasRenderBackend, HasRenderTree, HasTreeDrawer, ModuleConfiguration};
use gosub_interface::font::HasFontManager;
use gosub_interface::instance::InstanceId;
use gosub_renderer::draw::TreeDrawerImpl;
use gosub_rendering::render_tree::RenderTree;
use gosub_shared::geo::SizeU32;
use gosub_taffy::TaffyLayouter;

#[derive(Clone, Debug, PartialEq)]
pub struct Config;

impl HasCssSystem for Config {
    type CssSystem = Css3System;
}
impl HasDocument for Config {
    type Document = DocumentImpl<Self>;
    type DocumentFragment = DocumentFragmentImpl<Self>;
    type DocumentBuilder = DocumentBuilderImpl;
}

impl HasHtmlParser for Config {
    type HtmlParser = Html5Parser<'static, Self>;
}

impl HasLayouter for Config {
    type Layouter = TaffyLayouter;
    type LayoutTree = RenderTree<Self>;
}

impl HasRenderTree for Config {
    type RenderTree = RenderTree<Self>;
}

impl HasTreeDrawer for Config {
    type TreeDrawer = TreeDrawerImpl<Self>;
}

impl HasRenderBackend for Config {
    type RenderBackend = CairoBackend;
}

#[derive(Clone)]
struct GtkChromeHandle(UnboundedSender<Scene>);

impl ChromeHandle<Config> for GtkChromeHandle {
    fn draw_scene(&self, scene: gosub_interface::render_backend::Scene, _: SizeU32, _: InstanceId) {
        info!("Drawing scene");
        self.0.unbounded_send(scene).unwrap();
    }
}

impl HasChrome for Config {
    type ChromeHandle = GtkChromeHandle;
}

impl HasFontManager for Config {
    type FontManager = FontManager;
}

impl ModuleConfiguration for Config {}
