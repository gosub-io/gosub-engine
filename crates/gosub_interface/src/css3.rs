use crate::config::HasDocument;
use gosub_shared::async_executor::{WasmNotSend, WasmNotSendSync};
use gosub_shared::config::ParserConfig;
use gosub_shared::errors::CssResult;
use gosub_shared::node::NodeId;
use std::fmt::{Debug, Display};

/// Defines the origin of the stylesheet (or declaration)
#[derive(Debug, PartialEq, Clone, Copy)]
pub enum CssOrigin {
    /// Browser/user agent defined stylesheets
    UserAgent,
    /// Author defined stylesheets that are linked or embedded in the HTML files
    Author,
    /// User defined stylesheets that will override the author and user agent stylesheets (for instance, custom user styles or extensions)
    User,
}

/// Hover-sensitivity fingerprints for a set of stylesheets.
///
/// Records which element types/classes/ids appear in a `:hover` compound selector, so the
/// engine can skip style recalculation when the pointer moves between elements that no `:hover`
/// rule targets. Computed by the [`CssSystem`] (via [`CssSystem::hover_fingerprints`]) because
/// only the CSS implementation understands its own selector structure — the engine stays
/// agnostic of how selectors are represented.
#[derive(Default, Debug, Clone)]
pub struct HoverFingerprints {
    /// A bare `:hover` / `*:hover` rule exists — every node is hover-sensitive.
    pub has_universal: bool,
    /// Element type names that appear in a `:hover` compound.
    pub types: std::collections::HashSet<String>,
    /// Class names that appear in a `:hover` compound.
    pub classes: std::collections::HashSet<String>,
    /// Element ids that appear in a `:hover` compound.
    pub ids: std::collections::HashSet<String>,
}

/// The `CssSystem` trait is a trait that defines all things CSS3 that are used by other non-css3 crates. This is the main trait that
/// is used to parse CSS3 files. It contains sub elements like the Stylesheet trait that is used in for instance the Document trait.
pub trait CssSystem: Clone + Debug + 'static {
    type Stylesheet: CssStylesheet + WasmNotSendSync;

    type PropertyMap: CssPropertyMap<Self> + WasmNotSendSync;

    type Property: CssProperty<Self> + WasmNotSendSync;
    type Value: CssValue + WasmNotSendSync;

    /// Parses a string into a CSS3 stylesheet
    fn parse_str(str: &str, config: ParserConfig, origin: CssOrigin, source_url: &str) -> CssResult<Self::Stylesheet>;

    /// Returns the properties of a node
    /// If `None` is returned, the node is not renderable
    fn properties_from_node<C: HasDocument<CssSystem = Self>>(
        doc: &C::Document,
        id: NodeId,
        sheets: &[Self::Stylesheet],
    ) -> Option<Self::PropertyMap>;

    /// Returns the properties that apply to the `::before` / `::after` pseudo-element of `id`.
    /// `pseudo` is the pseudo-element name without colons (`"before"` or `"after"`). Returns
    /// `None` when no rule targets that pseudo-element (so no generated box should be created).
    /// The default implementation reports no pseudo-element styling.
    fn pseudo_properties_from_node<C: HasDocument<CssSystem = Self>>(
        _doc: &C::Document,
        _id: NodeId,
        _sheets: &[Self::Stylesheet],
        _pseudo: &str,
    ) -> Option<Self::PropertyMap> {
        None
    }

    fn load_default_useragent_stylesheet() -> Self::Stylesheet;

    /// Scan `sheets` and collect the [`HoverFingerprints`] — the element types/classes/ids that
    /// are the subject of a `:hover` rule. Lets the engine cheaply decide whether a hover change
    /// can affect styling without re-running selector matching.
    fn hover_fingerprints(sheets: &[Self::Stylesheet]) -> HoverFingerprints;
}

pub trait CssStylesheet: PartialEq + Debug {
    /// Returns the origin of the stylesheet
    fn origin(&self) -> CssOrigin;

    /// Returns the source URL of the stylesheet
    fn url(&self) -> &str;

    /// `@font-face` web fonts declared in this stylesheet, as
    /// `(family, source_urls, unicode_range)` tuples. The source URLs are unresolved
    /// (relative to the stylesheet's own URL); `unicode_range` is the raw descriptor or
    /// `None` when the face covers all code points.
    fn font_faces(&self) -> Vec<(String, Vec<String>, Option<String>)> {
        Vec::new()
    }
}

pub trait CssPropertyMap<S: CssSystem>: Default + Debug + WasmNotSend {
    fn insert_inherited(&mut self, name: &str, value: S::Property);

    fn insert(&mut self, name: &str, value: S::Property);

    fn get(&self, name: &str) -> Option<&S::Property>;

    fn get_mut(&mut self, name: &str) -> Option<&mut S::Property>;

    fn make_dirty(&mut self);

    fn iter(&self) -> impl Iterator<Item = (&str, &S::Property)> + '_;

    fn iter_mut(&mut self) -> impl Iterator<Item = (&str, &mut S::Property)> + '_;

    fn make_clean(&mut self);
    fn is_dirty(&self) -> bool;
}

pub trait CssProperty<S: CssSystem>: Debug + Display + Sized + From<S::Value> {
    fn compute_value(&mut self); // this should probably be removed

    fn unit_to_px(&self) -> f32;

    fn as_string(&self) -> Option<&str>;
    fn as_percentage(&self) -> Option<f32>;
    fn as_unit(&self) -> Option<(f32, &str)>;
    fn as_color(&self) -> Option<(f32, f32, f32, f32)>;

    fn parse_color(&self) -> Option<(f32, f32, f32, f32)>;

    fn as_number(&self) -> Option<f32>;
    fn as_list(&self) -> Option<&[S::Value]>;

    fn as_function(&self) -> Option<(&str, &[S::Value])>;

    fn is_none(&self) -> bool;
}

pub trait CssValue: Sized {
    fn new_string(value: &str) -> Self;
    fn new_percentage(value: f32) -> Self;
    fn new_unit(value: f32, unit: String) -> Self;
    fn new_color(r: f32, g: f32, b: f32, a: f32) -> Self;
    fn new_number(value: f32) -> Self;
    fn new_list(value: Vec<Self>) -> Self;

    fn unit_to_px(&self) -> f32;

    fn as_string(&self) -> Option<&str>;
    fn as_percentage(&self) -> Option<f32>;
    fn as_unit(&self) -> Option<(f32, &str)>;
    fn as_color(&self) -> Option<(f32, f32, f32, f32)>;
    fn as_number(&self) -> Option<f32>;
    fn as_list(&self) -> Option<&[Self]>;

    fn as_function(&self) -> Option<(&str, &[Self])>;

    fn is_comma(&self) -> bool;

    fn is_none(&self) -> bool;
}
