use crate::config::HasDrawComponents;
use crate::layout::LayoutTree;
use crate::render_backend::{ImgCache, NodeDesc, RenderBackend, WindowedEventLoop};
use gosub_shared::geo::{Point, SizeU32, FP};
use std::future::Future;
use std::sync::mpsc::Sender;
use url::Url;

pub trait TreeDrawer<C: HasDrawComponents> {
    type ImgCache: ImgCache<C::RenderBackend>;

    fn draw(
        &mut self,
        backend: &mut C::RenderBackend,
        data: &mut <C::RenderBackend as RenderBackend>::WindowData<'_>,
        size: SizeU32,
        el: &impl WindowedEventLoop<C>,
    ) -> bool;
    fn mouse_move(&mut self, backend: &mut C::RenderBackend, x: FP, y: FP) -> bool;

    fn scroll(&mut self, point: Point);
    fn from_url(
        url: Url,
        layouter: C::Layouter,
        debug: bool,
    ) -> impl Future<Output = gosub_shared::types::Result<Self>>
    where
        Self: Sized;

    fn from_source(
        // The initial url that the source was loaded from
        url: Url,
        // Actual loaded source HTML
        source_html: &str,
        // Layouter that renders the tree
        layouter: C::Layouter,
        // Debug flag
        debug: bool,
    ) -> impl Future<Output = gosub_shared::types::Result<Self>>
    where
        Self: Sized;

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

    fn reload(&mut self, el: impl WindowedEventLoop<C>);

    fn reload_from(&mut self, tree: C::RenderTree);
}
