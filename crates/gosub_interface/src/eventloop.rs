use crate::config::HasDrawComponents;
use crate::render_backend::ImageBuffer;
use crate::async_executor::WasmNotSendSync;
use crate::geo::SizeU32;
use url::Url;

pub trait EventLoopHandle<C: HasDrawComponents>: WasmNotSendSync + Clone + 'static {
    /// Request a redraw of the scene
    fn redraw(&self);

    /// Add an image to the cache
    fn add_img_cache(&self, url: Url, buf: ImageBuffer<C::RenderBackend>, size: Option<SizeU32>);

    /// Reload the instance from the given render tree
    fn reload_from(&self, rt: C::RenderTree);
}
