use crate::config::{HasDocument, HasDrawComponents, HasHtmlParser};
use crate::eventloop::EventLoopHandle;
use crate::layout::LayoutTree;
use crate::render_backend::{ImgCache, NodeDesc, RenderBackend};
use gosub_net::http::fetcher::Fetcher;
use gosub_shared::geo::{Point, SizeU32, FP};
use gosub_shared::types::Result;
use std::future::Future;
use std::sync::mpsc::Sender;
use std::sync::Arc;
use url::Url;

pub trait TreeDrawer<C: HasDrawComponents> {
    type ImgCache: ImgCache<C::RenderBackend>;

    fn draw(&mut self, size: SizeU32, el: &impl EventLoopHandle<C>) -> <C::RenderBackend as RenderBackend>::Scene;
    fn mouse_move(&mut self, x: FP, y: FP) -> bool;

    fn scroll(&mut self, point: Point);
    fn from_url(
        url: Url,
        layouter: C::Layouter,
        debug: bool,
    ) -> impl Future<Output = gosub_shared::types::Result<(Self, C::Document)>>
    where
        Self: Sized,
        C: HasDocument + HasHtmlParser;

    fn from_source(
        // The initial url that the source was loaded from
        url: Url,
        // Actual loaded source HTML
        source_html: &str,
        // Layouter that renders the tree
        layouter: C::Layouter,
        // Debug flag
        debug: bool,
    ) -> Result<(Self, C::Document)>
    where
        Self: Sized,
        C: HasDocument + HasHtmlParser;

    fn with_fetcher(
        // The initial url that the source was loaded from
        url: Url,
        // The fetcher that is used to load resources
        fetcher: Arc<Fetcher>,
        // Layouter that renders the tree
        layouter: C::Layouter,
        // Debug flag
        debug: bool,
    ) -> impl Future<Output = Result<(Self, C::Document)>>
    where
        Self: Sized,
        C: HasDocument + HasHtmlParser;

    fn clear_buffers(&mut self);
    fn toggle_debug(&mut self);

    fn select_element(&mut self, id: <C::LayoutTree as LayoutTree<C>>::NodeId);
    fn unselect_element(&mut self);

    fn info(&mut self, id: <C::LayoutTree as LayoutTree<C>>::NodeId, sender: Sender<NodeDesc>);
    fn send_nodes(&mut self, sender: Sender<NodeDesc>);

    fn set_needs_redraw(&mut self);

    fn get_img_cache(&mut self) -> &mut Self::ImgCache;

    fn make_dirty(&mut self);

    fn delete_scene(&mut self);

    fn reload(&mut self, el: impl EventLoopHandle<C>) -> impl Future<Output = Result<C::Document>> + 'static
    where
        C: HasDocument + HasHtmlParser;

    fn navigate(
        &mut self,
        url: Url,
        el: impl EventLoopHandle<C>,
    ) -> impl Future<Output = Result<C::Document>> + 'static
    where
        C: HasDocument + HasHtmlParser;

    fn reload_from(&mut self, tree: C::RenderTree);
}
