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
pub struct BrowserState<C: HasDocument> {
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

impl<C: HasDocument> Debug for BrowserState<C> {
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

pub trait BrowserConfiguration: HasDocument + Send + Sync + 'static
where
    Self::Document: Sync
{ }

static BROWSER_STATE: OnceLock<Arc<RwLock<dyn ErasedBrowserState>>> = OnceLock::new();

trait ErasedBrowserState: Debug + Send + Sync + 'static {
    fn as_any(&self) -> &dyn std::any::Any;
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any;
}

// impl Debug for ErasedBrowserState {
//     fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
//         f.debug_struct("ErasedBrowserState").finish()
//     }
// }

impl<C: BrowserConfiguration> ErasedBrowserState for BrowserState<C> where <C as HasDocument>::Document: Sync, <C as HasDocument>::Document: Send {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

pub fn init_browser_state<C: BrowserConfiguration>(state: BrowserState<C>) where <C as HasDocument>::Document: Sync, <C as HasDocument>::Document: Send {
    BROWSER_STATE.set(Arc::new(RwLock::new(state)))
        .expect("Browser state already initialized");
}

pub fn with_browser_state<F, R, C: BrowserConfiguration>(f: F) -> R
where
    F: FnOnce(&BrowserState<C>) -> R,
    C::Document: Sync,
{
    let state = BROWSER_STATE.get().expect("Browser state not initialized");
    let guard = state.read().unwrap();
    let concrete_state = guard.as_any().downcast_ref::<BrowserState<C>>()
        .expect("Incorrect BrowserState type");
    f(concrete_state)
}

pub fn with_browser_state_mut<F, R, C: BrowserConfiguration>(f: F) -> R
where
    F: FnOnce(&mut BrowserState<C>) -> R,
    C::Document: Sync,
{
    let state = BROWSER_STATE.get().expect("Browser state not initialized");
    let mut guard = state.write().unwrap();
    let concrete_state = guard.as_any_mut().downcast_mut::<BrowserState<C>>()
        .expect("Incorrect BrowserState type");
    f(concrete_state)
}


pub fn get_browser_state() -> Arc<RwLock<dyn ErasedBrowserState>> {
    BROWSER_STATE
        .get()
        .expect("Browser state not initialized")
        .clone()
}