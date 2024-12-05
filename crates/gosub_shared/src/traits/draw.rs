use crate::async_executor::WasmNotSend;
use crate::render_backend::layout::LayoutTree;
use crate::render_backend::{ImgCache, NodeDesc, Point, RenderBackend, SizeU32, WindowedEventLoop, FP};
use crate::traits::config::HasDrawComponents;
use std::future::Future;
use std::sync::mpsc::Sender;
use url::Url;

pub trait TreeDrawer<C: HasDrawComponents>: WasmNotSend + 'static {
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
    ) -> impl Future<Output = crate::types::Result<Self>> + WasmNotSend
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
