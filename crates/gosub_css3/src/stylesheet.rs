use core::fmt::Debug;
use core::slice;
use cow_utils::CowUtils;
use gosub_interface::css3::CssOrigin;
use gosub_shared::byte_stream::Location;
use gosub_shared::errors::CssError;
use gosub_shared::errors::CssResult;
use std::cmp::Ordering;
use std::fmt::Display;
use std::sync::Arc;

use crate::colors::{oklab_to_srgb, oklch_to_srgb, RgbColor};
use crate::matcher::index::{ElementKeys, SelectorIndex};
use crate::media_query::{media_environment, set_media_environment, MediaEnvironment, MediaQueryList};
use crate::supports::SupportsCondition;

/// Set the viewport (CSS px) used to resolve `vw`/`vh`/`vmin`/`vmax` for subsequent style
/// computations on this thread. The render flow calls this before building and laying out the
/// render tree so viewport units (including those inside `clamp()`) track the real window size
/// instead of a fixed fallback. Non-positive dimensions are ignored.
///
/// This updates the viewport half of the thread's [`MediaEnvironment`], which media queries
/// read too - the two must never disagree. Callers that also care about colour scheme or
/// resolution should build a whole environment and use [`set_media_environment`] instead.
pub fn set_layout_viewport(width: f32, height: f32) {
    if width > 0.0 && height > 0.0 {
        let mut env = media_environment();
        env.width = width;
        env.height = height;
        set_media_environment(env);
    }
}

/// The current viewport (CSS px) for resolving viewport-relative units on this thread.
fn layout_viewport() -> (f32, f32) {
    let env = media_environment();
    (env.width, env.height)
}

/// Severity of a CSS error
#[derive(Debug, PartialEq)]
pub enum Severity {
    /// A critical error that will prevent the stylesheet from being applied
    Error,
    /// A warning that will be displayed but will not prevent the stylesheet from being applied
    Warning,
    /// An information message that can be displayed to the user
    Info,
}

impl Display for Severity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Severity::Error => write!(f, "Error"),
            Severity::Warning => write!(f, "Warning"),
            Severity::Info => write!(f, "Info"),
        }
    }
}

/// Defines a CSS log during
#[derive(PartialEq)]
pub struct CssLog {
    /// Severity of the error
    pub severity: Severity,
    /// Error message
    pub message: String,
    /// Location of the error
    pub location: Location,
}

impl CssLog {
    #[must_use]
    pub fn log(severity: Severity, message: &str, location: Location) -> Self {
        Self {
            severity,
            message: message.to_string(),
            location,
        }
    }

    #[must_use]
    pub fn error(message: &str, location: Location) -> Self {
        Self {
            severity: Severity::Error,
            message: message.to_string(),
            location,
        }
    }

    #[must_use]
    pub fn warn(message: &str, location: Location) -> Self {
        Self {
            severity: Severity::Warning,
            message: message.to_string(),
            location,
        }
    }

    #[must_use]
    pub fn info(message: &str, location: Location) -> Self {
        Self {
            severity: Severity::Info,
            message: message.to_string(),
            location,
        }
    }
}

impl Display for CssLog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "[{}] ({}:{}): {}",
            self.severity, self.location.line, self.location.column, self.message
        )
    }
}

impl Debug for CssLog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "[{}] ({}:{}): {}",
            self.severity, self.location.line, self.location.column, self.message
        )
    }
}

/// A parsed `@font-face` rule: a logical font family and the (unresolved) URLs that
/// provide it. URLs are relative to the stylesheet's own URL until resolved by the consumer.
#[derive(Debug, PartialEq, Clone)]
pub struct FontFace {
    /// The `font-family` name this face provides (unquoted).
    pub family: String,
    /// Candidate `src: url(...)` targets in declared order.
    pub sources: Vec<String>,
    /// The raw `unicode-range` descriptor, if any (e.g. `"U+0000-00FF, U+0131"`). Used to
    /// pick the subset that covers the content; `None` means the face covers all code points.
    pub unicode_range: Option<String>,
}

/// An `@import` rule: another stylesheet whose rules belong ahead of this one's own.
///
/// Recorded unresolved. Fetching is the host's job (only it has a network stack and a URL
/// resolver); see [`CssStylesheet::splice_import`] for the merge back.
#[derive(Debug, PartialEq, Clone)]
pub struct ImportRule {
    /// The requested URL exactly as written, relative to the importing sheet's own URL.
    pub url: String,
    /// `layer` (as `Some(None)`) or `layer(name)` (as `Some(Some(name))`). Cascade layers are
    /// flattened by this engine, so this is recorded for fidelity but does not affect order.
    pub layer: Option<Option<String>>,
    /// `supports(...)` condition. The import is skipped entirely when it does not hold, so
    /// a sheet guarded on a feature this engine lacks is never fetched.
    pub supports: Option<SupportsCondition>,
    /// Trailing media query list. Every imported rule inherits it, so
    /// `@import "print.css" print;` cannot leak into screen rendering.
    pub media: Option<MediaQueryList>,
}

/// Defines a complete stylesheet with all its rules and the location where it was found
#[derive(Debug)]
pub struct CssStylesheet {
    /// List of rules found in this stylesheet
    pub rules: Vec<CssRule>,
    /// `@font-face` rules found in this stylesheet (web fonts).
    pub font_faces: Vec<FontFace>,
    /// `@import` rules, in source order, still unresolved.
    pub imports: Vec<ImportRule>,
    /// Whether any declaration in this sheet uses a viewport-relative unit (`vw`, `vh`,
    /// `vmin`, `vmax` and their `s`/`l`/`d` variants).
    ///
    /// Those resolve against the layout viewport *at style-computation time*, so a sheet
    /// that uses them has to be restyled on every resize, while one that does not can keep
    /// its cached computed values. Recorded once when the stylesheet is built.
    pub uses_viewport_units: bool,
    /// Origin of the stylesheet (user agent, author, user)
    pub origin: CssOrigin,
    /// Url or file path where the stylesheet was found
    pub url: String,
    /// Any issues during parsing of the stylesheet
    pub parse_log: Vec<CssLog>,
    /// Rule index by rightmost compound, built on first style computation and rebuilt when
    /// `rules` changed size since; see [`CssStylesheet::invalidate_index`] for other edits.
    pub(crate) index: parking_lot::RwLock<Option<SelectorIndex>>,
}

impl PartialEq for CssStylesheet {
    fn eq(&self, other: &Self) -> bool {
        self.rules == other.rules
            && self.font_faces == other.font_faces
            && self.imports == other.imports
            && self.uses_viewport_units == other.uses_viewport_units
            && self.origin == other.origin
            && self.url == other.url
            && self.parse_log == other.parse_log
    }
}

impl CssStylesheet {
    #[must_use]
    pub fn new(origin: CssOrigin, url: &str) -> Self {
        Self {
            rules: vec![],
            font_faces: vec![],
            imports: vec![],
            uses_viewport_units: false,
            origin,
            url: url.to_string(),
            parse_log: vec![],
            index: parking_lot::RwLock::new(None),
        }
    }

    /// Splice an imported stylesheet into this one, ahead of the rules already present.
    ///
    /// `@import` must precede every other rule, so an imported sheet's rules always cascade
    /// below the importing sheet's own; prepending in import order reproduces that. Repeated
    /// calls therefore have to append to the imported block rather than the front, which
    /// `insert_at` tracks for the caller.
    ///
    /// `media` is the import's own media query list; it is pushed onto every incoming rule so
    /// the condition travels with the rules rather than being lost at the seam. Font faces
    /// come along unconditionally - they are not media-scoped.
    pub fn splice_import(
        &mut self,
        imported: CssStylesheet,
        media: Option<&Arc<MediaQueryList>>,
        insert_at: usize,
    ) -> usize {
        let CssStylesheet {
            rules,
            font_faces,
            uses_viewport_units,
            ..
        } = imported;

        // An imported sheet's viewport-unit usage becomes the importing sheet's too: its
        // rules now live here, and the resize fingerprint is computed per sheet.
        self.uses_viewport_units |= uses_viewport_units;
        let count = rules.len();
        let rules = rules.into_iter().map(|mut rule| {
            if let Some(media) = media {
                // Outermost first: the import's condition gates everything inside it.
                rule.media.get_or_insert_with(Vec::new).insert(0, Arc::clone(media));
            }
            rule
        });
        self.rules.splice(insert_at..insert_at, rules);
        self.font_faces.extend(font_faces);
        // The index is keyed by rule position, so it has to be rebuilt.
        self.invalidate_index();
        insert_at + count
    }

    /// Drop the rule index so the next lookup rebuilds it. Call after editing `rules` in a
    /// way that keeps their number (reordering, replacing a rule); pushes and removals are
    /// detected by themselves.
    pub fn invalidate_index(&mut self) {
        *self.index.get_mut() = None;
    }

    /// The rules that can possibly match an element with these keys, in stylesheet order.
    pub(crate) fn candidate_rules(&self, keys: &ElementKeys<'_>) -> Vec<usize> {
        if let Some(index) = self
            .index
            .read()
            .as_ref()
            .filter(|index| index.rule_count() == self.rules.len())
        {
            return index.candidates(keys);
        }
        self.index
            .write()
            .insert(SelectorIndex::build(&self.rules))
            .candidates(keys)
    }
}

impl gosub_interface::css3::CssStylesheet for CssStylesheet {
    fn origin(&self) -> CssOrigin {
        self.origin
    }

    fn url(&self) -> &str {
        &self.url
    }

    fn font_faces(&self) -> Vec<(String, Vec<String>, Option<String>)> {
        self.font_faces
            .iter()
            .map(|f| (f.family.clone(), f.sources.clone(), f.unicode_range.clone()))
            .collect()
    }
}

/// A CSS rule, which contains a list of selectors and a list of declarations
#[derive(Debug, PartialEq, Clone)]
pub struct CssRule {
    /// Selectors that must match for the declarations to apply
    pub selectors: Vec<CssSelector>,
    /// Actual declarations that will be applied if the selectors match
    pub declarations: Vec<CssDeclaration>,
    /// The `@media` conditions enclosing this rule, outermost first - all of them must match
    /// before the rule applies. `None` for the overwhelmingly common unconditional rule, so
    /// the check costs a null test. Each list is shared by every rule in its block.
    ///
    /// Conditions are kept unevaluated so that a viewport change is a restyle, not a re-parse.
    pub media: Option<Vec<Arc<MediaQueryList>>>,
}

impl CssRule {
    #[must_use]
    pub fn selectors(&self) -> &Vec<CssSelector> {
        &self.selectors
    }

    #[must_use]
    pub fn declarations(&self) -> &Vec<CssDeclaration> {
        &self.declarations
    }

    /// Whether this rule's enclosing `@media` conditions hold in `env`. Unconditional rules
    /// always match.
    #[must_use]
    pub fn media_matches(&self, env: &MediaEnvironment) -> bool {
        self.media
            .as_ref()
            .is_none_or(|conditions| conditions.iter().all(|list| list.matches(env)))
    }
}

/// A CSS declaration, which contains a property, value and a flag for !important
#[derive(Debug, PartialEq, Clone)]
pub struct CssDeclaration {
    // Css property color
    pub property: String,
    // Raw values of the declaration. It is not calculated or converted in any way (ie: "red", "50px" etc.)
    // There can be multiple values  (ie:   "1px solid black" are split into 3 values)
    pub value: CssValue,
    // ie: !important
    pub important: bool,
}

#[derive(Debug, PartialEq, Clone)]
pub struct CssSelector {
    // List of parts that make up this selector
    pub parts: Vec<Vec<CssSelectorPart>>,
}

impl CssSelector {
    /// Generate specificity for this selector
    #[must_use]
    pub fn specificity(&self) -> Vec<Specificity> {
        self.parts
            .iter()
            .map(|part| Specificity::from(part.as_slice()))
            .collect()
    }
}

/// A CSS selector part: a type plus its value (e.g. type=Class, class="my-class")
#[derive(PartialEq, Clone, Default)]
pub enum CssSelectorPart {
    #[default]
    Universal,
    Attribute(Box<AttributeSelector>),
    Class(String),
    Id(String),
    PseudoClass(String),
    PseudoElement(String),
    Combinator(Combinator),
    Type(String),
    /// `:not(...)`, holding the selector list it negates. Matches when *none* of the inner
    /// selectors match the element.
    Not(Vec<Vec<CssSelectorPart>>),
}

#[derive(PartialEq, Clone, Default, Debug)]
pub struct AttributeSelector {
    pub name: String,
    pub matcher: MatcherType,
    pub value: String,
    pub case_insensitive: bool,
}

#[derive(Debug, PartialEq, Clone)]
pub enum Combinator {
    Descendant,
    Child,
    NextSibling,
    SubsequentSibling,
    Column,
    Namespace,
}

impl Display for Combinator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Combinator::Descendant => write!(f, " "),
            Combinator::Child => write!(f, ">"),
            Combinator::NextSibling => write!(f, "+"),
            Combinator::SubsequentSibling => write!(f, "~"),
            Combinator::Column => write!(f, "||"),
            Combinator::Namespace => write!(f, "|"),
        }
    }
}

impl Debug for CssSelectorPart {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CssSelectorPart::Universal => {
                write!(f, "*")
            }
            CssSelectorPart::Attribute(selector) => {
                write!(
                    f,
                    "[{} {} {} {}]",
                    selector.name, selector.matcher, selector.value, selector.case_insensitive
                )
            }
            CssSelectorPart::Class(name) => {
                write!(f, ".{name}")
            }
            CssSelectorPart::Id(name) => {
                write!(f, "#{name}")
            }
            CssSelectorPart::PseudoClass(name) => {
                write!(f, ":{name}")
            }
            CssSelectorPart::PseudoElement(name) => {
                write!(f, "::{name}")
            }
            CssSelectorPart::Combinator(combinator) => {
                write!(f, "'{combinator}'")
            }
            CssSelectorPart::Type(name) => {
                write!(f, "{name}")
            }
            CssSelectorPart::Not(inner) => {
                write!(f, ":not(")?;
                for (i, compound) in inner.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    for part in compound {
                        write!(f, "{part:?}")?;
                    }
                }
                write!(f, ")")
            }
        }
    }
}

/// The type of a selector part
#[derive(Debug, PartialEq, Clone, Default)]
pub enum CssSelectorType {
    Universal, // '*'
    #[default]
    Type, //  ul, a, h1, etc
    Attribute, // [type ~= "text" i]  (name, matcher, value, flags)
    Class,     // .myclass
    Id,        // #myid
    PseudoClass, // :hover, :active
    PseudoElement, // ::first-child
    Combinator,
}

/// Represents which type of matcher is used (in case of an attribute selector type)
#[derive(Default, PartialEq, Clone, Debug)]
pub enum MatcherType {
    #[default]
    None, // No matcher
    Equals,         // Equals
    Includes,       // Must include
    DashMatch,      // Must start with
    PrefixMatch,    // Must begin with
    SuffixMatch,    // Must ends with
    SubstringMatch, // Must contain
}

impl Display for MatcherType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MatcherType::None => write!(f, ""),
            MatcherType::Equals => write!(f, "="),
            MatcherType::Includes => write!(f, "~="),
            MatcherType::DashMatch => write!(f, "|="),
            MatcherType::PrefixMatch => write!(f, "^="),
            MatcherType::SuffixMatch => write!(f, "$="),
            MatcherType::SubstringMatch => write!(f, "*="),
        }
    }
}

/// Defines the specificity for a selector
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct Specificity(u32, u32, u32);

impl Specificity {
    #[must_use]
    pub const fn new(a: u32, b: u32, c: u32) -> Self {
        Self(a, b, c)
    }

    #[must_use]
    pub const fn id_count(&self) -> u32 {
        self.0
    }

    #[must_use]
    pub const fn class_count(&self) -> u32 {
        self.1
    }

    #[must_use]
    pub const fn element_count(&self) -> u32 {
        self.2
    }
}

/// Whether a serialized pseudo-class contributes no specificity at all - `:where()`, and only
/// `:where()` (Selectors L4 §17).
///
/// Compares bytes rather than lowercasing: this runs inside `match_selector`, once per element
/// per candidate rule, so it must not allocate.
fn is_zero_specificity_pseudo(name: &str) -> bool {
    let bytes = name.as_bytes();
    bytes.len() > 6 && bytes[..6].eq_ignore_ascii_case(b"where(")
}

impl From<&[CssSelectorPart]> for Specificity {
    fn from(parts: &[CssSelectorPart]) -> Self {
        let mut id_count = 0;
        let mut class_count = 0;
        let mut element_count = 0;
        for part in parts {
            match part {
                CssSelectorPart::Id(_) => {
                    id_count += 1;
                }
                CssSelectorPart::Class(_) => {
                    class_count += 1;
                }
                CssSelectorPart::Type(_) => {
                    element_count += 1;
                }
                // An attribute selector counts as a class, same as `.foo` (Selectors L4 §17).
                CssSelectorPart::Attribute(_) => {
                    class_count += 1;
                }
                CssSelectorPart::PseudoClass(name) => {
                    // `:where()` contributes nothing whatever it contains - that is the entire
                    // point of it - while every other pseudo-class counts as a class.
                    //
                    // Known gap: `:is()` and `:has()` should take the specificity of their most
                    // specific argument. Unlike `:not`, which has its own structured variant,
                    // they are stored here as serialized text, so that is not computable without
                    // giving them the same treatment. Counting them as one class is the
                    // pre-Selectors-4 behaviour and errs low rather than high.
                    if !is_zero_specificity_pseudo(name) {
                        class_count += 1;
                    }
                }
                // Legacy single-colon `:before`/`:after` are re-classified as pseudo-elements
                // during AST conversion, so they land here and count as elements too.
                CssSelectorPart::PseudoElement(_) => {
                    element_count += 1;
                }
                // Selectors L4 §17: `:not()` contributes nothing itself, but its most specific
                // argument counts as if it were written in place of the `:not()`.
                CssSelectorPart::Not(inner) => {
                    if let Some(most) = inner.iter().map(|parts| Specificity::from(parts.as_slice())).max() {
                        id_count += most.id_count();
                        class_count += most.class_count();
                        element_count += most.element_count();
                    }
                }
                _ => {}
            }
        }
        Specificity::new(id_count, class_count, element_count)
    }
}

impl PartialOrd for Specificity {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Specificity {
    fn cmp(&self, other: &Self) -> Ordering {
        match self.0.cmp(&other.0) {
            Ordering::Greater => Ordering::Greater,
            Ordering::Less => Ordering::Less,
            Ordering::Equal => match self.1.cmp(&other.1) {
                Ordering::Greater => Ordering::Greater,
                Ordering::Less => Ordering::Less,
                Ordering::Equal => match self.2.cmp(&other.2) {
                    Ordering::Greater => Ordering::Greater,
                    Ordering::Less => Ordering::Less,
                    Ordering::Equal => Ordering::Equal,
                },
            },
        }
    }
}

/// Actual CSS value, can be a color, length, percentage, string or unit. Some relative values will be computed
/// from other values (ie: Percent(50) will convert to Length(100) when the parent width is 200)
#[derive(Debug, Clone, PartialEq)]
pub enum CssValue {
    None,
    Color(RgbColor),
    Zero,
    Number(f32),
    Percentage(f32),
    String(String),
    Unit(f32, String),
    Function(String, Vec<CssValue>),
    Initial,
    Inherit,
    Comma,
    List(Vec<CssValue>),
}

/// The viewport-relative length units, which resolve against the layout viewport when a
/// declaration is computed rather than when it is used. Kept in step with the `unit_to_px`
/// match below.
const VIEWPORT_UNITS: &[&str] = &["vw", "svw", "lvw", "dvw", "vh", "svh", "lvh", "dvh", "vmin", "vmax"];

impl CssValue {
    /// Whether this value (or anything nested inside it) is expressed in a viewport-relative
    /// unit, and so has to be recomputed when the viewport resizes.
    #[must_use]
    pub fn uses_viewport_units(&self) -> bool {
        match self {
            CssValue::Unit(_, unit) => VIEWPORT_UNITS.iter().any(|u| unit.eq_ignore_ascii_case(u)),
            // `calc()` alone keeps its body as raw text (see `parse_ast_node`), so the units
            // inside it never become `Unit` values and the arm above cannot see them. Scan the
            // text instead. Other functions - `clamp()` included - parse their arguments into
            // real values and are handled by the recursion below.
            //
            // Only `calc()` is scanned, deliberately: a blanket string scan would also match
            // `url(https://example.org/100vw.png)` or `content: "100vw"`, and every one of those
            // false positives costs a full style recompute on each resize.
            CssValue::Function(name, args) if name.eq_ignore_ascii_case("calc") => args.iter().any(|arg| match arg {
                CssValue::String(body) => text_uses_viewport_units(body),
                other => other.uses_viewport_units(),
            }),
            CssValue::Function(_, args) => args.iter().any(CssValue::uses_viewport_units),
            CssValue::List(values) => values.iter().any(CssValue::uses_viewport_units),
            _ => false,
        }
    }
}

/// Whether raw value text contains a viewport-relative unit token, for the `calc()` body that
/// never gets parsed into [`CssValue::Unit`].
///
/// Splits on anything that cannot appear in a unit token, then strips the numeric part, so
/// `100vw` yields `vw` while `overview` (no leading digits) is left whole and matches nothing.
fn text_uses_viewport_units(text: &str) -> bool {
    text.split(|c: char| !c.is_ascii_alphanumeric() && c != '.')
        .any(|word| {
            let unit = word.trim_start_matches(|c: char| c.is_ascii_digit() || c == '.');
            // A bare identifier is not a unit: it has to follow a number.
            unit.len() != word.len() && VIEWPORT_UNITS.iter().any(|u| unit.eq_ignore_ascii_case(u))
        })
}

impl Display for CssValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CssValue::None => write!(f, "none"),
            CssValue::Color(col) => {
                write!(
                    f,
                    "#{:02x}{:02x}{:02x}{:02x}",
                    col.r as u8, col.g as u8, col.b as u8, col.a as u8
                )
            }
            CssValue::Zero => write!(f, "0"),
            CssValue::Number(num) => write!(f, "{num}"),
            CssValue::Percentage(p) => write!(f, "{p}%"),
            CssValue::String(s) => write!(f, "{s}"),
            CssValue::Unit(val, unit) => write!(f, "{val}{unit}"),
            CssValue::Function(name, args) => {
                write!(f, "{name}(")?;
                for (i, arg) in args.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{arg}")?;
                }
                write!(f, ")")
            }
            CssValue::Initial => write!(f, "initial"),
            CssValue::Inherit => write!(f, "inherit"),
            CssValue::Comma => write!(f, ","),
            CssValue::List(v) => {
                write!(f, "List(")?;
                for (i, value) in v.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{value}")?;
                }
                write!(f, ")")
            }
        }
    }
}

impl CssValue {
    #[must_use]
    pub fn to_color(&self) -> Option<RgbColor> {
        match self {
            CssValue::Color(col) => Some(*col),
            CssValue::String(s) => Some(RgbColor::from(s.as_str())),
            CssValue::Function(name, args) => parse_css_color_function(name, args),
            _ => None,
        }
    }

    #[must_use]
    pub fn unit_to_px(&self) -> f32 {
        match self {
            CssValue::Unit(val, unit) => match unit.as_str() {
                "px" => *val,
                "em" => *val * 16.0,
                "rem" => *val * 16.0,
                // Absolute physical units - 1in = 96px
                "pt" => *val * (96.0 / 72.0),
                "pc" => *val * (96.0 / 6.0),
                "in" => *val * 96.0,
                "cm" => *val * (96.0 / 2.54),
                "mm" => *val * (96.0 / 25.4),
                "q" => *val * (96.0 / 101.6),
                // Viewport units - resolved against the current layout viewport (CSS px),
                // falling back to 1280×800 until the render flow sets the real size.
                "vw" | "svw" | "lvw" | "dvw" => *val * layout_viewport().0 / 100.0,
                "vh" | "svh" | "lvh" | "dvh" => *val * layout_viewport().1 / 100.0,
                "vmin" => {
                    let (w, h) = layout_viewport();
                    *val * w.min(h) / 100.0
                }
                "vmax" => {
                    let (w, h) = layout_viewport();
                    *val * w.max(h) / 100.0
                }
                _ => *val,
            },
            CssValue::String(value) => {
                if value.ends_with("px") {
                    value.trim_end_matches("px").parse::<f32>().unwrap_or(0.0)
                } else if value.ends_with("rem") {
                    value.trim_end_matches("rem").parse::<f32>().unwrap_or(0.0) * 16.0
                } else if value.ends_with("em") {
                    value.trim_end_matches("em").parse::<f32>().unwrap_or(0.0) * 16.0
                } else if value.ends_with("__qem") {
                    value.trim_end_matches("__qem").parse::<f32>().unwrap_or(0.0) * 16.0
                } else {
                    0.0
                }
            }
            _ => 0.0,
        }
    }

    #[must_use]
    pub fn from_vec(mut value: Vec<Self>) -> Self {
        match value.len() {
            0 => Self::None,
            1 => value.swap_remove(0),
            _ => Self::List(value),
        }
    }

    #[must_use]
    pub fn to_slice(&self) -> &[Self] {
        match self {
            Self::List(l) => l,
            this => slice::from_ref(this),
        }
    }

    #[must_use]
    pub fn into_vec(self) -> Vec<Self> {
        match self {
            Self::List(l) => l,
            this => vec![this],
        }
    }

    /// Converts a CSS AST node to a CSS value
    pub fn parse_ast_node(node: crate::node::Node) -> CssResult<CssValue> {
        match node.node_type {
            crate::node::NodeType::Ident { value } => Ok(CssValue::String(value)),
            crate::node::NodeType::Number { value } => {
                if value == 0.0 {
                    // Zero is a special case since we need to do some pattern matching once in a while, and
                    // this is not possible (anymore) with floating point 0.0 it seems
                    Ok(CssValue::Zero)
                } else {
                    Ok(CssValue::Number(value))
                }
            }
            crate::node::NodeType::Percentage { value } => Ok(CssValue::Percentage(value)),
            crate::node::NodeType::Dimension { value, unit } => Ok(CssValue::Unit(value, unit)),
            crate::node::NodeType::String { value } => Ok(CssValue::String(value)),
            crate::node::NodeType::Hash { mut value } => {
                value.insert(0, '#');
                Ok(CssValue::Color(RgbColor::from(value.as_str())))
            }
            // Keep the operator character (e.g. `/` in `16 / 9` or `font: 14px/1.5`)
            // as a string so it can match a `/` literal in a value grammar. Discarding
            // it (as `None`) makes `<ratio>` and other slash-delimited grammars unmatchable.
            crate::node::NodeType::Operator(value) => Ok(CssValue::String(value)),
            crate::node::NodeType::Calc { expr } => {
                // Preserve the raw body of calc(...) so the layout engine can evaluate it later.
                let body = match expr.node_type {
                    crate::node::NodeType::Raw { value } => value,
                    _ => String::new(),
                };
                Ok(CssValue::Function("calc".to_string(), vec![CssValue::String(body)]))
            }
            crate::node::NodeType::Url { url } => {
                Ok(CssValue::Function("url".to_string(), vec![CssValue::String(url)]))
            }
            crate::node::NodeType::Function { name, arguments } => {
                let mut list = vec![];
                for node in arguments {
                    list.push(CssValue::parse_ast_node(node)?);
                }
                // Color functions (rgb/rgba/hsl/hsla/oklch/…) collapse to a concrete `Color`
                // at parse time. This lets `<color>` syntax matching (which only recognises
                // `Color`/hex) accept them inside shorthands like `border`/`background`, and
                // avoids re-parsing the function on every style lookup.
                if is_color_function(&name) {
                    if let Some(color) = parse_css_color_function(&name, &list) {
                        return Ok(CssValue::Color(color));
                    }
                }
                Ok(CssValue::Function(name, list))
            }

            crate::node::NodeType::Comma => Ok(CssValue::Comma),

            _ => Err(CssError::new(
                format!("Cannot convert node to CssValue: {node:?}").as_str(),
            )),
        }
    }

    /// Parses a string into a CSS value or list of css values
    pub fn parse_str(value: &str) -> CssResult<CssValue> {
        match value {
            "initial" => return Ok(CssValue::Initial),
            "inherit" => return Ok(CssValue::Inherit),
            "none" => return Ok(CssValue::None),
            "" => return Ok(CssValue::String(String::new())),
            _ => {}
        }

        if let Ok(num) = value.parse::<f32>() {
            return Ok(CssValue::Number(num));
        }

        // Color values
        if value.starts_with("color(") && value.ends_with(')') {
            return Ok(CssValue::Color(RgbColor::from(
                value[6..value.len() - 1].to_string().as_str(),
            )));
        }

        // Percentages
        if value.ends_with('%') {
            if let Ok(num) = value[0..value.len() - 1].parse::<f32>() {
                return Ok(CssValue::Percentage(num));
            }
        }

        // units. If the value starts with a number and ends with some non-numerical
        let mut split_index = None;
        for (index, char) in value.chars().enumerate() {
            if char.is_alphabetic() {
                split_index = Some(index);
                break;
            }
        }
        if let Some(index) = split_index {
            let (number_part, unit_part) = value.split_at(index);
            if let Ok(number) = number_part.parse::<f32>() {
                return Ok(CssValue::Unit(number, unit_part.to_string()));
            }
        }

        Ok(CssValue::String(value.to_string()))
    }
}

/// Parse a CSS color function like `oklch()`, `oklab()`, or `color()` into an RgbColor.
///
/// Handles the CSS Color Level 4 space-separated syntax, including an optional alpha
/// separated by `/` (represented as `CssValue::None` after the CSS parser processes it).
/// True for CSS functional color notations that `parse_css_color_function` can resolve.
fn is_color_function(name: &str) -> bool {
    matches!(
        name.cow_to_ascii_lowercase().as_ref(),
        "rgb" | "rgba" | "hsl" | "hsla" | "oklch" | "oklab" | "color"
    )
}

fn parse_css_color_function(name: &str, args: &[CssValue]) -> Option<RgbColor> {
    // Collect numeric/percentage/none arguments, skipping the `/` delimiter (stored as None)
    // and any string tokens (like the color-space name in `color(srgb ...)`).
    // CSS `none` keyword means "missing value" = 0.
    let nums: Vec<f32> = args
        .iter()
        .filter_map(|v| match v {
            CssValue::Number(n) => Some(*n),
            CssValue::Percentage(p) => Some(*p),
            CssValue::Zero => Some(0.0),
            CssValue::String(s) if s.eq_ignore_ascii_case("none") => Some(0.0),
            _ => None,
        })
        .collect();

    // Helper to resolve an L (lightness) argument: percentage 0-100 → 0.0-1.0, decimal as-is.
    let resolve_l = |raw: f32, is_pct: bool| -> f32 {
        if is_pct {
            raw / 100.0
        } else {
            raw
        }
    };

    // Detect whether each positional arg was given as a percentage.
    let is_pct: Vec<bool> = args
        .iter()
        .filter_map(|v| match v {
            CssValue::Number(_) | CssValue::Zero => Some(false),
            CssValue::Percentage(_) => Some(true),
            CssValue::String(s) if s.eq_ignore_ascii_case("none") => Some(false),
            _ => None,
        })
        .collect();

    match name.cow_to_ascii_lowercase().as_ref() {
        "oklch" if nums.len() >= 3 => {
            let l = resolve_l(nums[0], *is_pct.first().unwrap_or(&false));
            // Chroma: percentage 0-100 maps to ~0-0.4 max chroma.
            let c = if *is_pct.get(1).unwrap_or(&false) {
                nums[1] / 100.0 * 0.4
            } else {
                nums[1]
            };
            let h = nums[2];
            let alpha = nums
                .get(3)
                .copied()
                .map(|a| {
                    if *is_pct.get(3).unwrap_or(&false) {
                        a / 100.0 * 255.0
                    } else {
                        a * 255.0
                    }
                })
                .unwrap_or(255.0);
            let (r, g, b) = oklch_to_srgb(l, c, h);
            Some(RgbColor::new(r, g, b, alpha))
        }
        "oklab" if nums.len() >= 3 => {
            let l = resolve_l(nums[0], *is_pct.first().unwrap_or(&false));
            let a_ok = if *is_pct.get(1).unwrap_or(&false) {
                nums[1] / 100.0 * 0.4
            } else {
                nums[1]
            };
            let b_ok = if *is_pct.get(2).unwrap_or(&false) {
                nums[2] / 100.0 * 0.4
            } else {
                nums[2]
            };
            let alpha = nums
                .get(3)
                .copied()
                .map(|a| {
                    if *is_pct.get(3).unwrap_or(&false) {
                        a / 100.0 * 255.0
                    } else {
                        a * 255.0
                    }
                })
                .unwrap_or(255.0);
            let (r, g, b) = oklab_to_srgb(l, a_ok, b_ok);
            Some(RgbColor::new(r, g, b, alpha))
        }
        // color(srgb R G B) or color(display-p3 R G B) - treat as linear/sRGB for now.
        "color" if nums.len() >= 3 => {
            // First element of args is the color space name (a String), skip it.
            let alpha = nums
                .get(3)
                .copied()
                .map(|a| {
                    if *is_pct.get(3).unwrap_or(&false) {
                        a / 100.0 * 255.0
                    } else {
                        a * 255.0
                    }
                })
                .unwrap_or(255.0);
            Some(RgbColor::new(nums[0] * 255.0, nums[1] * 255.0, nums[2] * 255.0, alpha))
        }
        // rgb(R G B) / rgba(R G B A). Channels are 0-255 numbers or 0%-100% percentages.
        "rgb" | "rgba" if nums.len() >= 3 => {
            let chan = |i: usize| -> f32 {
                if *is_pct.get(i).unwrap_or(&false) {
                    nums[i] / 100.0 * 255.0
                } else {
                    nums[i]
                }
            };
            Some(RgbColor::new(chan(0), chan(1), chan(2), parse_alpha(&nums, &is_pct, 3)))
        }
        // hsl(H S% L%) / hsla(...). Hue in degrees; saturation/lightness as percentages.
        "hsl" | "hsla" if nums.len() >= 3 => {
            let (r, g, b) = hsl_to_srgb(nums[0], nums[1] / 100.0, nums[2] / 100.0);
            Some(RgbColor::new(r, g, b, parse_alpha(&nums, &is_pct, 3)))
        }
        _ => None,
    }
}

/// Resolves an optional alpha argument at `idx` into the 0-255 range. A bare number is a
/// 0-1 ratio; a percentage is 0-100. Missing alpha is fully opaque.
fn parse_alpha(nums: &[f32], is_pct: &[bool], idx: usize) -> f32 {
    nums.get(idx)
        .copied()
        .map(|a| {
            if *is_pct.get(idx).unwrap_or(&false) {
                a / 100.0 * 255.0
            } else {
                a * 255.0
            }
        })
        .unwrap_or(255.0)
}

/// Converts HSL (hue in degrees, saturation/lightness in 0-1) to sRGB channels in 0-255.
fn hsl_to_srgb(h: f32, s: f32, l: f32) -> (f32, f32, f32) {
    let h = h.rem_euclid(360.0) / 360.0;
    if s <= 0.0 {
        let v = l * 255.0;
        return (v, v, v);
    }
    let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
    let p = 2.0 * l - q;
    let hue = |mut t: f32| -> f32 {
        if t < 0.0 {
            t += 1.0;
        }
        if t > 1.0 {
            t -= 1.0;
        }
        let c = if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 1.0 / 2.0 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        };
        c * 255.0
    };
    (hue(h + 1.0 / 3.0), hue(h), hue(h - 1.0 / 3.0))
}

impl gosub_interface::css3::CssValue for CssValue {
    fn new_string(value: &str) -> Self {
        CssValue::String(value.to_string())
    }

    fn new_percentage(value: f32) -> Self {
        CssValue::Percentage(value)
    }

    fn new_unit(value: f32, unit: String) -> Self {
        CssValue::Unit(value, unit)
    }

    fn new_color(r: f32, g: f32, b: f32, a: f32) -> Self {
        CssValue::Color(RgbColor::new(r, g, b, a))
    }

    fn new_number(value: f32) -> Self {
        CssValue::Number(value)
    }

    fn new_list(value: Vec<Self>) -> Self {
        CssValue::List(value)
    }

    fn unit_to_px(&self) -> f32 {
        self.unit_to_px()
    }

    fn as_string(&self) -> Option<&str> {
        if let CssValue::String(str) = &self {
            Some(str)
        } else {
            None
        }
    }

    fn as_percentage(&self) -> Option<f32> {
        if let CssValue::Percentage(percent) = &self {
            Some(*percent)
        } else {
            None
        }
    }

    fn as_unit(&self) -> Option<(f32, &str)> {
        if let CssValue::Unit(value, unit) = &self {
            Some((*value, unit))
        } else {
            None
        }
    }

    fn as_color(&self) -> Option<(f32, f32, f32, f32)> {
        if let CssValue::Color(color) = &self {
            Some((color.r, color.g, color.b, color.a))
        } else {
            None
        }
    }

    fn as_number(&self) -> Option<f32> {
        match self {
            CssValue::Number(num) => Some(*num),
            // Bare `0` (no unit) is a valid zero value for any numeric property.
            CssValue::Zero => Some(0.0),
            _ => None,
        }
    }

    fn as_list(&self) -> Option<&[Self]> {
        if let CssValue::List(list) = &self {
            Some(list)
        } else {
            None
        }
    }

    fn as_function(&self) -> Option<(&str, &[Self])> {
        if let CssValue::Function(name, args) = &self {
            Some((name.as_str(), args))
        } else {
            None
        }
    }

    fn is_comma(&self) -> bool {
        matches!(self, CssValue::Comma)
    }

    fn is_none(&self) -> bool {
        matches!(self, CssValue::None)
    }
}

#[cfg(test)]
mod test {
    use std::vec;

    use super::*;

    /// `calc()` keeps its body as raw text, so the units in it never become `CssValue::Unit`.
    /// Missing them leaves `uses_viewport_units` false, the style fingerprint then omits the
    /// viewport, and a resize never invalidates the values resolved against the old one.
    #[test]
    fn calc_bodies_are_scanned_for_viewport_units() {
        let calc = |body: &str| {
            CssValue::Function("calc".to_string(), vec![CssValue::String(body.to_string())]).uses_viewport_units()
        };

        assert!(calc("100vw - 2rem"));
        assert!(calc("100% - 10DVH"), "unit matching is case-insensitive");
        assert!(calc("(50vmin + 1px) / 2"));
        assert!(!calc("100% - 2rem"));
    }

    /// Only `calc()` is scanned. A blanket string scan would fire on these, and each false
    /// positive costs a full style recompute on every resize.
    #[test]
    fn other_functions_do_not_scan_raw_text() {
        let url = CssValue::Function(
            "url".to_string(),
            vec![CssValue::String("https://example.org/100vw.png".to_string())],
        );
        assert!(!url.uses_viewport_units());

        assert!(!CssValue::String("100vw".to_string()).uses_viewport_units());
    }

    /// A viewport unit has to follow a number; a bare identifier that merely contains those
    /// letters is not one.
    #[test]
    fn identifiers_containing_unit_letters_are_not_units() {
        let calc = |body: &str| {
            CssValue::Function("calc".to_string(), vec![CssValue::String(body.to_string())]).uses_viewport_units()
        };
        assert!(!calc("var(--overview) + 1px"));
        assert!(!calc("vh"));
    }

    /// Functions whose arguments are parsed properly still work through the recursion.
    #[test]
    fn parsed_function_arguments_still_match() {
        let clamp = CssValue::Function(
            "clamp".to_string(),
            vec![
                CssValue::Unit(1.0, "rem".to_string()),
                CssValue::Unit(50.0, "vw".to_string()),
                CssValue::Unit(9.0, "rem".to_string()),
            ],
        );
        assert!(clamp.uses_viewport_units());
    }

    #[test]
    fn test_css_rule() {
        let rule = CssRule {
            selectors: vec![CssSelector {
                parts: vec![vec![CssSelectorPart::Type("h1".to_string())]],
            }],
            declarations: vec![CssDeclaration {
                property: "color".to_string(),
                value: CssValue::String("red".to_string()),
                important: false,
            }],
            media: None,
        };

        assert_eq!(rule.selectors().len(), 1);
        let part = rule
            .selectors()
            .first()
            .unwrap()
            .parts
            .first()
            .unwrap()
            .first()
            .unwrap();

        assert_eq!(part, &CssSelectorPart::Type("h1".to_string()));
        assert_eq!(rule.declarations().len(), 1);
        assert_eq!(rule.declarations().first().unwrap().property, "color");
    }

    /// Everything that carries specificity, at each of the three levels.
    ///
    /// Pseudo-classes, pseudo-elements and attribute selectors were all being ignored, so
    /// `div:hover` scored the same as bare `div` and could lose a cascade it should win.
    #[test]
    fn specificity_counts_pseudos_and_attributes() {
        let spec = |parts: Vec<CssSelectorPart>| Specificity::from(parts.as_slice());

        // `div:hover` - one element, one class-level pseudo-class.
        assert_eq!(
            spec(vec![
                CssSelectorPart::Type("div".into()),
                CssSelectorPart::PseudoClass("hover".into()),
            ]),
            Specificity::new(0, 1, 1)
        );

        // `p::after` - two element-level components. Legacy `:after` converts to a
        // pseudo-element upstream, so it lands on this same arm.
        assert_eq!(
            spec(vec![
                CssSelectorPart::Type("p".into()),
                CssSelectorPart::PseudoElement("after".into()),
            ]),
            Specificity::new(0, 0, 1 + 1)
        );

        // `[type="text"]` counts as a class.
        assert_eq!(
            spec(vec![CssSelectorPart::Attribute(Box::new(AttributeSelector {
                name: "type".into(),
                matcher: MatcherType::Equals,
                value: "text".into(),
                case_insensitive: false,
            }))]),
            Specificity::new(0, 1, 0)
        );
    }

    /// `:not()` contributes the specificity of its most specific argument, and now that
    /// pseudo-classes count, that argument may itself be one.
    #[test]
    fn specificity_of_not_sees_inner_pseudo_classes() {
        let selector = vec![
            CssSelectorPart::Type("div".into()),
            CssSelectorPart::Not(vec![vec![CssSelectorPart::PseudoClass("hover".into())]]),
        ];
        assert_eq!(Specificity::from(selector.as_slice()), Specificity::new(0, 1, 1));
    }

    /// `:where()` exists precisely so that it adds nothing, however specific its argument.
    #[test]
    fn where_pseudo_class_adds_no_specificity() {
        assert_eq!(
            Specificity::from([CssSelectorPart::PseudoClass("where(#id .cls)".into())].as_slice()),
            Specificity::new(0, 0, 0)
        );
        // Case-insensitively, and without mistaking a differently-named pseudo-class for it.
        assert_eq!(
            Specificity::from([CssSelectorPart::PseudoClass("WHERE(.a)".into())].as_slice()),
            Specificity::new(0, 0, 0)
        );
        assert_eq!(
            Specificity::from([CssSelectorPart::PseudoClass("wherever".into())].as_slice()),
            Specificity::new(0, 1, 0)
        );
    }

    #[test]
    fn test_specificity() {
        let selector = CssSelector {
            parts: vec![vec![
                CssSelectorPart::Type("h1".to_string()),
                CssSelectorPart::Class("myclass".to_string()),
                CssSelectorPart::Id("myid".to_string()),
            ]],
        };

        let specificity = selector.specificity();
        assert_eq!(specificity, vec![Specificity::new(1, 1, 1)]);

        let selector = CssSelector {
            parts: vec![vec![
                CssSelectorPart::Type("h1".to_string()),
                CssSelectorPart::Class("myclass".to_string()),
            ]],
        };

        let specificity = selector.specificity();
        assert_eq!(specificity, vec![Specificity::new(0, 1, 1)]);

        let selector = CssSelector {
            parts: vec![vec![CssSelectorPart::Type("h1".to_string())]],
        };

        let specificity = selector.specificity();
        assert_eq!(specificity, vec![Specificity::new(0, 0, 1)]);

        let selector = CssSelector {
            parts: vec![vec![
                CssSelectorPart::Class("myclass".to_string()),
                CssSelectorPart::Class("otherclass".to_string()),
            ]],
        };

        let specificity = selector.specificity();
        assert_eq!(specificity, vec![Specificity::new(0, 2, 0)]);
    }

    #[test]
    fn test_specificity_ordering() {
        let specificity1 = Specificity::new(1, 1, 1);
        let specificity2 = Specificity::new(0, 1, 1);
        let specificity3 = Specificity::new(0, 0, 1);
        let specificity4 = Specificity::new(0, 2, 0);
        let specificity5 = Specificity::new(1, 0, 0);
        let specificity6 = Specificity::new(1, 2, 1);
        let specificity7 = Specificity::new(1, 1, 2);
        let specificity8 = Specificity::new(2, 1, 1);

        assert!(specificity1 > specificity2);
        assert!(specificity2 > specificity3);
        assert!(specificity3 < specificity4);
        assert!(specificity4 < specificity5);
        assert!(specificity5 < specificity6);
        assert!(specificity6 > specificity7);
        assert!(specificity7 < specificity8);
    }

    #[test]
    fn rgb_hsl_color_functions() {
        // rgba(): channels 0-255, alpha 0-1 → 0-255.
        let c = parse_css_color_function(
            "rgba",
            &[
                CssValue::Number(14.0),
                CssValue::Comma,
                CssValue::Number(42.0),
                CssValue::Comma,
                CssValue::Number(54.0),
                CssValue::Comma,
                CssValue::Number(0.5),
            ],
        )
        .expect("rgba should parse");
        assert_eq!((c.r, c.g, c.b), (14.0, 42.0, 54.0));
        assert!((c.a - 127.5).abs() < 0.5);

        // rgb() without alpha is fully opaque.
        let c = parse_css_color_function(
            "rgb",
            &[CssValue::Number(255.0), CssValue::Number(0.0), CssValue::Number(0.0)],
        )
        .unwrap();
        assert_eq!((c.r, c.g, c.b, c.a), (255.0, 0.0, 0.0, 255.0));

        // hsl(0 100% 50%) == red.
        let c = parse_css_color_function(
            "hsl",
            &[
                CssValue::Number(0.0),
                CssValue::Percentage(100.0),
                CssValue::Percentage(50.0),
            ],
        )
        .unwrap();
        assert!((c.r - 255.0).abs() < 1.0 && c.g < 1.0 && c.b < 1.0, "hsl red got {c:?}");

        // A color function collapses to CssValue::Color at AST conversion time.
        assert!(is_color_function("rgba") && !is_color_function("calc"));
    }
}
