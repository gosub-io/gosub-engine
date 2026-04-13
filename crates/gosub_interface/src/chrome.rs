use crate::config::HasRenderBackend;
use crate::geo::SizeU32;
use crate::instance::InstanceId;
use crate::render_backend::RenderBackend;

/// A `ChromeHandle` is a trait that allows a potential instance of the engine to call back to the Chrome/Useragent
/// this can include drawing the scene
pub trait ChromeHandle<C: HasRenderBackend>: Send + Clone {
    fn draw_scene(&self, scene: <C::RenderBackend as RenderBackend>::Scene, size: SizeU32, instance: InstanceId);
}
