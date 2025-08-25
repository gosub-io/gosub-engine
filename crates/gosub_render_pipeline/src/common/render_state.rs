use std::fmt::Debug;
use std::sync::{Arc, OnceLock, RwLock};
use gosub_interface::config::HasDocument;
use crate::common::geo::Rect;
use crate::layouter::LayoutElementId;
use crate::tiler::TileList;

#[derive(Debug)]
pub enum WireframeState {
    None,
    Only,
    Both,
}

/// Things that can change in the browser is stored in this structure. It keeps the current rendering pipeline (in the form of a layer_list),
/// and some things that we can control, or is controlled by the user (like current_hovered_element).
pub struct RenderState<C: HasDocument> {
    /// List of layers that will be visible are set to true
    pub visible_layer_list: Vec<bool>,
    /// Defines if we need to draw wireframes, or the actual content, or both
    pub wireframed: WireframeState,
    /// Just show the hovered debug node in wireframe
    pub debug_hover: bool,
    /// Show the tile grid
    pub show_tilegrid: bool,
    /// When set, this is the element that is currently hovered upon
    pub current_hovered_element: Option<LayoutElementId>,
    /// Current viewport offset + size
    pub viewport: Rect,
    /// Main document that is currently being rendered
    pub document: Arc<C::Document>,
    /// LayerList that is currently being rendered
    pub tile_list: Option<RwLock<TileList<C>>>,
    /// Scale factor for DPI
    pub dpi_scale_factor: f32,
}

impl<C: HasDocument> Debug for RenderState<C> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BrowserState")
            .field("visible_layer_list", &self.visible_layer_list)
            .field("wireframed", &self.wireframed)
            .field("debug_hover", &self.debug_hover)
            .field("show_tilegrid", &self.show_tilegrid)
            .field("current_hovered_element", &self.current_hovered_element)
            .field("viewport", &self.viewport)
            .field("dpi_scale_factor", &self.dpi_scale_factor)
            .finish()
    }
}

pub trait AnyRenderState: Debug + Send + Sync + 'static {
    fn as_any(&self) -> &dyn std::any::Any;
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any;
}

static RENDER_STATE: OnceLock<Arc<RwLock<dyn AnyRenderState>>> = OnceLock::new();

pub fn init_render_state(state: Arc<RwLock<dyn AnyRenderState>>) {
    RENDER_STATE.set(state).expect("Render state already set");
}

fn get_render_state() -> Arc<RwLock<dyn AnyRenderState>> {
    RENDER_STATE.get().expect("Render state not initialized").clone()
}

pub fn with_render_state<F, R, C: 'static>(f: F) -> R
where
    F: FnOnce(&RenderState<C>) -> R,
    C: HasDocument + 'static + Sync + Send,
{
    let binding = get_render_state();
    let guard = binding.read().unwrap();
    let state = guard
        .as_any()
        .downcast_ref::<RenderState<C>>()
        .expect("Incorrect RenderState type");
    f(state)
}

pub fn with_render_state_mut<F, R, C: 'static>(f: F) -> R
where
    F: FnOnce(&mut RenderState<C>) -> R,
    C: HasDocument + 'static + Sync + Send,
{
    let binding = get_render_state();
    let mut guard = binding.write().unwrap();
    let state = guard
        .as_any_mut()
        .downcast_mut::<RenderState<C>>()
        .expect("Incorrect RenderState type");
    f(state)
}


#[macro_export]
macro_rules! with_render_state {
    ($cfg:ty, $state:ident => $body:block) => {
        $crate::render_state::with_render_state::<_, _, $cfg>(|$state| $body)
    };
}

#[macro_export]
macro_rules! with_render_state_mut {
    ($cfg:ty, $state:ident => $body:block) => {
        $crate::render_state::with_render_state_mut::<_, _, $cfg>(|$state| $body)
    };
}