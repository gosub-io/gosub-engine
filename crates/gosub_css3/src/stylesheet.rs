use core::fmt::Debug;
use core::slice;
use cow_utils::CowUtils;
use gosub_interface::css3::CssOrigin;
use gosub_shared::byte_stream::Location;
use gosub_shared::errors::CssError;
use gosub_shared::errors::CssResult;
use gosub_shared::node::NodeId;
use std::cmp::Ordering;
use std::fmt::Display;
use std::sync::Arc;
use std::sync::OnceLock;

use crate::colors::{ColorSyntax, CssColor, PredefinedSpace, RgbColor};
use crate::matcher::bloom::ancestor_keys;
use crate::matcher::expansion::{expand_declarations, ExpandedDeclaration};
use crate::matcher::index::{ElementKeys, SelectorIndex};
use crate::media_query::{media_environment, set_media_environment, MediaEnvironment, MediaQueryList};
use crate::supports::SupportsCondition;
use crate::tokenizer::NumberKind;

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

static PREFERS_DARK: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Set the user's colour-scheme preference, consumed by `light-dark()` and by rules under
/// `@media (prefers-color-scheme: …)`. Process-wide, like the rest of the UA preferences.
pub fn set_prefers_dark(dark: bool) {
    PREFERS_DARK.store(dark, std::sync::atomic::Ordering::Relaxed);
}

pub fn prefers_dark() -> bool {
    PREFERS_DARK.load(std::sync::atomic::Ordering::Relaxed)
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
    /// The tree scope this sheet was parsed into: `None` for the document, or the shadow root
    /// whose shadow tree holds the `<style>` / `<link>` that produced it.
    ///
    /// A sheet only applies inside its own scope. The two exceptions are the shadow tree's
    /// deliberate reach outwards - `:host` onto the host element, and `::slotted()` onto the
    /// light-DOM nodes projected into its slots - both of which live in the tree *outside*.
    /// User-agent sheets ignore scope entirely and apply everywhere.
    pub scope: Option<NodeId>,
    /// Url or file path where the stylesheet was found.
    ///
    /// Shared rather than owned outright: every declaration the cascade records keeps the URL it
    /// came from, and on a page of a few thousand elements that was a few hundred thousand
    /// copies of the same string.
    pub url: std::sync::Arc<str>,
    /// Any issues during parsing of the stylesheet
    pub parse_log: Vec<CssLog>,
    /// Cascade layers this sheet declares, by full dotted name, in the order they were first
    /// declared - which is the order that decides which of them wins (css-cascade-5 §6.4).
    ///
    /// A layer is named here whether it was given rules or only announced by a bare
    /// `@layer a, b;`, because announcing it is how a sheet fixes the order up front, before
    /// either block is written. A rule points into this list by index.
    pub layers: Vec<String>,
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
    /// A stylesheet with no rules, for the cases where a sheet could not be produced and the
    /// caller has to carry on without one.
    #[must_use]
    pub fn empty(origin: CssOrigin, url: &str) -> Self {
        CssStylesheet {
            rules: Vec::new(),
            font_faces: Vec::new(),
            imports: Vec::new(),
            uses_viewport_units: false,
            origin,
            scope: None,
            url: url.into(),
            parse_log: Vec::new(),
            layers: Vec::new(),
            index: parking_lot::RwLock::new(None),
        }
    }

    #[must_use]
    pub fn new(origin: CssOrigin, url: &str) -> Self {
        Self {
            rules: vec![],
            font_faces: vec![],
            imports: vec![],
            uses_viewport_units: false,
            origin,
            scope: None,
            url: url.into(),
            parse_log: vec![],
            layers: vec![],
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

    /// Write the rules that can possibly match an element with these keys into `out`, in
    /// stylesheet order. The buffer is the caller's so that a lookup costs no allocation.
    pub(crate) fn candidate_rules(&self, keys: &ElementKeys<'_>, out: &mut Vec<usize>) {
        if let Some(index) = self
            .index
            .read()
            .as_ref()
            .filter(|index| index.rule_count() == self.rules.len())
        {
            index.candidates(keys, out);
            return;
        }
        self.index
            .write()
            .insert(SelectorIndex::build(&self.rules))
            .candidates(keys, out);
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
#[derive(Debug, Clone)]
pub struct CssRule {
    /// Selectors that must match for the declarations to apply
    pub selectors: Vec<CssSelector>,
    /// Actual declarations that will be applied if the selectors match.
    ///
    /// Editing these once the rule is in a document invalidates [`CssRule::expanded`], which is
    /// built from them and kept. Nothing does: the parser fills them in before the rule reaches
    /// a stylesheet, and the CSSOM rewrites the `style` attribute's text, which is parsed into a
    /// sheet of its own.
    pub declarations: Vec<CssDeclaration>,
    /// The `@media` conditions enclosing this rule, outermost first - all of them must match
    /// before the rule applies. `None` for the overwhelmingly common unconditional rule, so
    /// the check costs a null test. Each list is shared by every rule in its block.
    ///
    /// Conditions are kept unevaluated so that a viewport change is a restyle, not a re-parse.
    pub media: Option<Vec<Arc<MediaQueryList>>>,
    /// The cascade layer this rule sits in, as an index into its sheet's
    /// [`CssStylesheet::layers`]. `None` for a rule outside every layer, which for a normal
    /// declaration is the strongest place to be.
    pub layer: Option<u32>,
    /// The declarations validated and expanded, built the first time an element needs them;
    /// see [`CssRule::expanded`].
    expanded: OnceLock<Vec<ExpandedDeclaration>>,
}

/// The expansion is a function of the declarations and nothing else, so it plays no part in
/// whether two rules are the same rule.
impl PartialEq for CssRule {
    fn eq(&self, other: &Self) -> bool {
        self.selectors == other.selectors
            && self.declarations == other.declarations
            && self.media == other.media
            && self.layer == other.layer
    }
}

impl CssRule {
    /// A rule as the parser builds it, with the declaration expansion still to be done.
    #[must_use]
    pub fn new(
        selectors: Vec<CssSelector>,
        declarations: Vec<CssDeclaration>,
        media: Option<Vec<Arc<MediaQueryList>>>,
        layer: Option<u32>,
    ) -> Self {
        Self {
            selectors,
            declarations,
            media,
            layer,
            expanded: OnceLock::new(),
        }
    }

    #[must_use]
    pub fn selectors(&self) -> &Vec<CssSelector> {
        &self.selectors
    }

    /// Whether any selector of this rule places a condition on an ancestor of the element. A
    /// rule that does not can be matched without an ancestor filter, which is what keeps a page
    /// whose sheets are all single-compound selectors from building one at all.
    pub(crate) fn asks_about_ancestors(&self) -> bool {
        self.selectors.iter().any(CssSelector::asks_about_ancestors)
    }

    #[must_use]
    pub fn declarations(&self) -> &Vec<CssDeclaration> {
        &self.declarations
    }

    /// The rule's declarations, each validated against its property definition and expanded
    /// into the longhands it sets, in the same order as [`CssRule::declarations`].
    ///
    /// None of that depends on the element the rule is being applied to, so it is done once
    /// here rather than once per matched element. It is built on first use rather than when the
    /// rule is parsed, because rules arrive after parsing too: `@import` splices whole sheets
    /// in, and an element's `style` attribute is a sheet built on its own.
    #[must_use]
    pub fn expanded(&self) -> &[ExpandedDeclaration] {
        self.expanded.get_or_init(|| expand_declarations(&self.declarations))
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
    /// The complex selectors of the list (`a, b` is two), each as its sequence of parts.
    parts: Vec<Vec<CssSelectorPart>>,
    /// Specificity of each entry in `parts`, computed once when the selector is built.
    ///
    /// A selector's specificity depends on nothing but the selector, so it used to be counted
    /// afresh every time the selector matched an element: a rule that matched a thousand
    /// elements walked its parts a thousand times for the same answer. Both vectors are
    /// private so the two cannot drift apart.
    specificity: Vec<Specificity>,
    /// What each entry in `parts` needs of the element's ancestors, hashed here rather than per
    /// element; see [`crate::matcher::bloom`]. Nearly every entry is empty, and an empty boxed
    /// slice owns nothing, so a sheet of selectors that ask nothing of an ancestor costs one
    /// allocation each.
    ancestor_keys: Box<[Box<[u32]>]>,
    /// Whether any entry of `ancestor_keys` is non-empty, so that a rule can be matched without
    /// an ancestor filter being built at all.
    asks_about_ancestors: bool,
}

impl CssSelector {
    #[must_use]
    pub fn new(parts: Vec<Vec<CssSelectorPart>>) -> Self {
        let specificity = parts.iter().map(|part| Specificity::from(part.as_slice())).collect();
        let ancestor_keys: Box<[Box<[u32]>]> = parts.iter().map(|complex| ancestor_keys(complex)).collect();
        let asks_about_ancestors = ancestor_keys.iter().any(|keys| !keys.is_empty());
        Self {
            parts,
            specificity,
            ancestor_keys,
            asks_about_ancestors,
        }
    }

    /// What every complex selector of this list needs of the element's ancestors, in the same
    /// order as [`CssSelector::parts`].
    pub(crate) fn ancestor_keys(&self) -> &[Box<[u32]>] {
        &self.ancestor_keys
    }

    /// Whether this selector places any condition at all on an ancestor. `false` means the
    /// ancestor filter would answer "maybe" whatever it held, so it need not exist.
    pub(crate) fn asks_about_ancestors(&self) -> bool {
        self.asks_about_ancestors
    }

    /// The complex selectors making up this selector list.
    #[must_use]
    pub fn parts(&self) -> &[Vec<CssSelectorPart>] {
        &self.parts
    }

    /// The specificity of each complex selector, in the same order as [`CssSelector::parts`].
    #[must_use]
    pub fn specificity(&self) -> &[Specificity] {
        &self.specificity
    }

    /// Each complex selector paired with its specificity.
    pub fn complex(&self) -> impl Iterator<Item = (&[CssSelectorPart], Specificity)> {
        self.parts
            .iter()
            .zip(&self.specificity)
            .map(|(parts, specificity)| (parts.as_slice(), *specificity))
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
    /// `:host` (as `None`) or `:host(<selector>)` (as `Some`). Matches the element a shadow
    /// tree hangs off, and only from that tree's own stylesheets - the host itself lives in
    /// the outer tree, so this is one of the two ways a shadow sheet reaches outwards.
    Host(Option<Vec<Vec<CssSelectorPart>>>),
    /// `::slotted(<selector>)`. Matches a light-DOM node projected into one of this shadow
    /// tree's slots, and only the directly assigned node - never its descendants.
    Slotted(Vec<Vec<CssSelectorPart>>),
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

/// Writes a comma-separated selector list, as the functional pseudo-classes print their argument.
fn write_selector_list(f: &mut std::fmt::Formatter<'_>, list: &[Vec<CssSelectorPart>]) -> std::fmt::Result {
    for (i, compound) in list.iter().enumerate() {
        if i > 0 {
            write!(f, ", ")?;
        }
        for part in compound {
            write!(f, "{part:?}")?;
        }
    }
    Ok(())
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
                write_selector_list(f, inner)?;
                write!(f, ")")
            }
            CssSelectorPart::Host(None) => write!(f, ":host"),
            CssSelectorPart::Host(Some(inner)) => {
                write!(f, ":host(")?;
                write_selector_list(f, inner)?;
                write!(f, ")")
            }
            CssSelectorPart::Slotted(inner) => {
                write!(f, "::slotted(")?;
                write_selector_list(f, inner)?;
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
                // `:host` counts as a pseudo-class, plus the specificity of its argument
                // (Scoping §6.1); `:host` alone is (0,1,0) and `:host(.a)` is (0,2,0).
                CssSelectorPart::Host(inner) => {
                    class_count += 1;
                    if let Some(most) = inner
                        .iter()
                        .flatten()
                        .map(|parts| Specificity::from(parts.as_slice()))
                        .max()
                    {
                        id_count += most.id_count();
                        class_count += most.class_count();
                        element_count += most.element_count();
                    }
                }
                // `::slotted()` is a pseudo-element, and its argument counts too.
                CssSelectorPart::Slotted(inner) => {
                    element_count += 1;
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
    Color(CssColor),
    Zero,
    /// A number, with the type flag css-syntax gave it. `<integer>` reads the flag rather than
    /// asking whether the value happens to be whole, so `1e1` is a `<number>` and not an
    /// `<integer>` even though it is ten.
    Number(f64, NumberKind),
    Percentage(f64),
    String(String),
    Unit(f64, String),
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
            // A `calc()` body arrives parsed, so its units are `Unit` values and the recursion
            // below sees them. The text arm covers a `calc()` built by hand with a raw body,
            // which only tests do, and is scanned rather than ignored so such a value still
            // reports its units.
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

/// Escape what a quoted CSS string cannot carry literally: the quote that delimits it, and the
/// backslash that does the escaping.
fn escape_url(url: &str) -> std::borrow::Cow<'_, str> {
    if !url.contains(['"', '\\']) {
        return std::borrow::Cow::Borrowed(url);
    }
    std::borrow::Cow::Owned(url.cow_replace('\\', "\\\\").cow_replace('"', "\\\"").into_owned())
}

impl Display for CssValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CssValue::None => write!(f, "none"),
            // `rgb()` / `rgba()`, not `#rrggbbaa` - see the `Display` on `RgbColor`. The hex
            // form is not a serialization any CSS consumer expects; it was a debug rendering.
            CssValue::Color(col) => write!(f, "{col}"),
            CssValue::Zero => write!(f, "0"),
            // Values are carried at f64 so a sum does not accumulate error on the way, but they
            // are *printed* at f32 width, which is the precision the value actually has by the
            // time anything reads it. Printing at f64 width reports digits that are an artifact
            // of binary fractions rather than of the value: `calc(0.1 + 0.2)` would serialize as
            // `0.30000000000000004`.
            CssValue::Number(num, _) => write!(f, "{}", *num as f32),
            CssValue::Percentage(p) => write!(f, "{}%", *p as f32),
            CssValue::String(s) => write!(f, "{s}"),
            CssValue::Unit(val, unit) => write!(f, "{}{unit}", *val as f32),
            // A `url()` always serializes with its argument quoted, whatever the author wrote.
            // The unquoted `url(x)` form is a token the CSS syntax defines, not a string, and
            // writing it back out unquoted loses the distinction for anything containing a
            // character the unquoted form cannot carry.
            CssValue::Function(name, args) if name.eq_ignore_ascii_case("url") => match args.as_slice() {
                [CssValue::String(url)] => write!(f, "url(\"{}\")", escape_url(url)),
                _ => write!(f, "url()"),
            },
            // A parenthesized group inside a math expression is held as a call with no name,
            // so it writes back out as the `( ... )` the author wrote rather than as a
            // `calc( ... )` that means the same thing but is not what the serialization rules
            // ask for.
            CssValue::Function(name, args) => {
                write!(f, "{name}(")?;
                // The argument list carries its own separators: the parser keeps each `,` as a
                // `CssValue::Comma` among the arguments. Joining with ", " as well emitted both,
                // so `min(50%, 100px)` came back as `min(50%, ,, 100px)`. Write a space only
                // where one belongs - after a comma, or between two arguments written side by
                // side as in `translate(1px 2px)` - and never before a comma.
                for (i, arg) in args.iter().enumerate() {
                    let bracket = |value: &CssValue, which: &str| matches!(value, CssValue::String(s) if s == which);
                    let after_open = i > 0 && bracket(&args[i - 1], "[");
                    if i > 0 && !matches!(arg, CssValue::Comma) && !after_open && !bracket(arg, "]") {
                        write!(f, " ")?;
                    }
                    write!(f, "{arg}")?;
                }
                write!(f, ")")
            }
            CssValue::Initial => write!(f, "initial"),
            CssValue::Inherit => write!(f, "inherit"),
            CssValue::Comma => write!(f, ","),
            // A list is how several values for one property are held - `margin: 1px 2px`, or the
            // single-element list `resolve_functions` wraps its result in. `List(1px, 2px)` was a
            // debug rendering that reached anything reading a computed value as text; the CSS is
            // the values themselves, separated the way they were written.
            CssValue::List(v) => {
                let bracket = |value: &CssValue, which: &str| matches!(value, CssValue::String(s) if s == which);
                for (i, value) in v.iter().enumerate() {
                    // No space inside a line-name list: `[a b]`, `[]`.
                    let after_open = i > 0 && bracket(&v[i - 1], "[");
                    if i > 0 && !matches!(value, CssValue::Comma) && !after_open && !bracket(value, "]") {
                        write!(f, " ")?;
                    }
                    write!(f, "{value}")?;
                }
                Ok(())
            }
        }
    }
}

impl CssValue {
    #[must_use]
    pub fn to_color(&self) -> Option<RgbColor> {
        match self {
            CssValue::Color(col) => Some(col.to_rgb()),
            // Fallible on purpose: a string that is not a colour (`none`, `no-repeat`, any
            // keyword that lands in a colour slot) must leave the property unset rather than
            // resolve to `RgbColor`'s opaque-black default and paint over the element.
            CssValue::String(s) => RgbColor::try_from_str(s.as_str()),
            CssValue::Function(name, args) => parse_css_color_function(name, args).map(|color| color.to_rgb()),
            _ => None,
        }
    }

    /// The length in px, narrowed to `f32` because that is the width the layout and paint
    /// side works in - see `gosub_interface::css3::CssProperty`. Values are carried at `f64`
    /// up to here, which is where css-values says the arithmetic happens.
    #[must_use]
    pub fn unit_to_px(&self) -> f32 {
        self.unit_to_px_f64() as f32
    }

    pub(crate) fn unit_to_px_f64(&self) -> f64 {
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
                //
                // The small (`sv*`), large (`lv*`) and dynamic (`dv*`) viewports all resolve to
                // that same size on purpose. They differ only where the UA has interfaces that
                // dynamically expand and retract - a phone browser's address bar - and css-values-4
                // says that a UA without them has all three equal to the initial containing block.
                // The engine has no such chrome, so they are equal here by the spec rather than by
                // omission. Give them their own sizes if an embedder ever grows retractable UI.
                "vw" | "svw" | "lvw" | "dvw" => *val * f64::from(layout_viewport().0) / 100.0,
                "vh" | "svh" | "lvh" | "dvh" => *val * f64::from(layout_viewport().1) / 100.0,
                "vmin" => {
                    let (w, h) = layout_viewport();
                    *val * f64::from(w.min(h)) / 100.0
                }
                "vmax" => {
                    let (w, h) = layout_viewport();
                    *val * f64::from(w.max(h)) / 100.0
                }
                _ => *val,
            },
            CssValue::String(value) => {
                if value.ends_with("px") {
                    value.trim_end_matches("px").parse::<f64>().unwrap_or(0.0)
                } else if value.ends_with("rem") {
                    value.trim_end_matches("rem").parse::<f64>().unwrap_or(0.0) * 16.0
                } else if value.ends_with("em") {
                    value.trim_end_matches("em").parse::<f64>().unwrap_or(0.0) * 16.0
                } else if value.ends_with("__qem") {
                    value.trim_end_matches("__qem").parse::<f64>().unwrap_or(0.0) * 16.0
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
            crate::node::NodeType::Number { value, kind } => {
                if value == 0.0 {
                    // Zero is a special case since we need to do some pattern matching once in a while, and
                    // this is not possible (anymore) with floating point 0.0 it seems.
                    //
                    // It keeps no type flag, so `z-index: 0.0` is accepted where the spelling says
                    // it should not be. Every `<length>`, `<time>` and `<angle>` arm recognises a
                    // bare zero through this variant, so giving it a flag is a wider change than
                    // the one case it would fix.
                    Ok(CssValue::Zero)
                } else {
                    Ok(CssValue::Number(value, kind))
                }
            }
            crate::node::NodeType::Percentage { value } => Ok(CssValue::Percentage(value)),
            // A unit identifier is ASCII case-insensitive, so it is folded here rather than at
            // every point that reads one. Both the syntax matcher (which looks a unit up in a
            // lowercase table) and `unit_to_px` (which matches it literally) took the author's
            // spelling as written, so `width: 1PX` was rejected as an unknown unit.
            crate::node::NodeType::Dimension { value, unit } => {
                Ok(CssValue::Unit(value, unit.cow_to_ascii_lowercase().into_owned()))
            }
            crate::node::NodeType::String { value } => Ok(CssValue::String(value)),
            crate::node::NodeType::Hash { mut value } => {
                value.insert(0, '#');
                Ok(CssValue::Color(RgbColor::from(value.as_str()).into()))
            }
            // Keep the operator character (e.g. `/` in `16 / 9` or `font: 14px/1.5`)
            // as a string so it can match a `/` literal in a value grammar. Discarding
            // it (as `None`) makes `<ratio>` and other slash-delimited grammars unmatchable.
            crate::node::NodeType::Operator { value, .. } => Ok(CssValue::String(value)),
            // A `calc()` body is its arguments, and is simplified as far as it can be without
            // knowing the element: the arithmetic and the absolute lengths, which is why
            // `calc(1in + 1px)` is stored as `calc(97px)`. `em`, the viewport units and
            // percentages all need something only the cascade or layout has, so they survive to
            // be finished in `resolve_computed`. A body this cannot make sense of (an
            // unsubstituted `var()`, say) is kept as the values it was written with.
            crate::node::NodeType::Calc { tokens } => {
                let mut body = Vec::with_capacity(tokens.len());
                for token in tokens {
                    body.push(CssValue::parse_ast_node(token)?);
                }
                Ok(reduce_function("calc".to_string(), body))
            }
            crate::node::NodeType::Url { url } => {
                Ok(CssValue::Function("url".to_string(), vec![CssValue::String(url)]))
            }
            crate::node::NodeType::Function { name, arguments } => {
                let mut list = vec![];
                for node in arguments {
                    list.push(CssValue::parse_ast_node(node)?);
                }
                Ok(reduce_function(name, list))
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

        if let Ok(num) = value.parse::<f64>() {
            // This reads text, so it can see the spelling css-syntax keys the type flag on.
            let kind = if value.contains(['.', 'e', 'E']) {
                NumberKind::Number
            } else {
                NumberKind::Integer
            };
            return Ok(CssValue::Number(num, kind));
        }

        // Percentages
        if value.ends_with('%') {
            if let Ok(num) = value[0..value.len() - 1].parse::<f64>() {
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
            if let Ok(number) = number_part.parse::<f64>() {
                return Ok(CssValue::Unit(number, unit_part.to_string()));
            }
        }

        Ok(CssValue::String(value.to_string()))
    }
}

/// A function call reduced as far as the parse can take it.
///
/// Colour functions (`rgb`/`hsl`/`oklch`/…) collapse to a concrete `Color`, which is what lets
/// `<color>` syntax matching (it only recognises `Color`/hex) accept one inside a shorthand like
/// `border` or `background`, and saves re-parsing the function on every style lookup. A math
/// function is simplified as far as knowing no element allows: what reduces serializes as
/// `calc()` - `min(1px, 2px)` is `calc(1px)` - and what does not (`min(1em, 2px)`, before there
/// is a font-size) stays exactly as written. Anything else is the call it was.
///
/// It is shared with `var()` substitution, which happens after the value was parsed and so
/// leaves behind a call that never went past this point: `rgb(var(--r) 0 0)` would stay a
/// function where `rgb(1 0 0)` is a `Color`. css-variables-1 §3 says the substituted value is
/// read as if the author had written it, so it is reduced here the same way.
pub(crate) fn reduce_function(name: String, args: Vec<CssValue>) -> CssValue {
    if is_color_function(&name) {
        if let Some(color) = parse_css_color_function(&name, &args) {
            return CssValue::Color(color);
        }
    }
    if let Some(reduced) =
        crate::functions::calc::evaluate_call(&name, &args, &crate::functions::calc::Units::none(), false)
    {
        return reduced;
    }
    CssValue::Function(name, args)
}

/// Parse a CSS color function like `oklch()`, `oklab()`, or `color()` into an RgbColor.
///
/// Handles the CSS Color Level 4 space-separated syntax, including an optional alpha
/// separated by `/` (represented as `CssValue::None` after the CSS parser processes it).
/// True for CSS functional color notations that `parse_css_color_function` can resolve.
pub(crate) fn is_color_function(name: &str) -> bool {
    matches!(
        name.cow_to_ascii_lowercase().as_ref(),
        "rgb" | "rgba" | "hsl" | "hsla" | "hwb" | "lab" | "lch" | "oklch" | "oklab" | "color"
    )
}

/// A `calc()` component of a colour function, reduced to the number it came down to. Anything
/// else is returned as it was, so the caller can see what is still unresolved.
fn reduce_color_component(value: &CssValue) -> CssValue {
    let CssValue::Function(name, args) = value else {
        return value.clone();
    };
    if !name.eq_ignore_ascii_case("calc") {
        return value.clone();
    }
    crate::functions::calc::evaluate(args, &crate::functions::calc::Units::none(), true)
        .unwrap_or_else(|| value.clone())
}

/// One component of a colour, in the units its notation uses.
///
/// `None` is a component written `none`, which css-color-4 §12.2 calls *missing* and which is
/// not the same as zero. `scale` is what a percentage means here: 255 for an sRGB channel, 100
/// for a percentage that stays a percentage, 1 for a `color()` component.
fn color_component(value: &CssValue, scale: f64) -> Option<Option<f64>> {
    match value {
        // css-values-4 §10.9: NaN becomes zero and an infinity clamps to the end of the range.
        CssValue::Number(number, _) => Some(Some(finite(*number, scale))),
        CssValue::Zero => Some(Some(0.0)),
        // Scaled by one factor rather than divided and multiplied: `30%` of an axis that is
        // itself a percentage has to come out exactly 30, not 30.00000191.
        CssValue::Percentage(percentage) => Some(Some(*percentage * (scale / 100.0))),
        CssValue::String(word) if word.eq_ignore_ascii_case("none") => Some(None),
        _ => None,
    }
}

/// A hue, which may be written as a plain number or as any angle unit (css-color-4 §7).
fn color_hue(value: &CssValue) -> Option<Option<f64>> {
    match value {
        CssValue::Unit(angle, unit) => {
            let degrees = match unit.as_str() {
                "deg" => *angle,
                "grad" => *angle * 0.9,
                "rad" => angle.to_degrees(),
                "turn" => *angle * 360.0,
                _ => return None,
            };
            Some(Some(finite(degrees, 0.0)))
        }
        _ => color_component(value, 360.0),
    }
}

/// Bring a component that is not a number back into the range it belongs to: NaN is zero, and
/// an infinity is as far as the component goes (css-values-4 §10.9).
fn finite(value: f64, scale: f64) -> f64 {
    if value.is_nan() {
        0.0
    } else if value.is_infinite() {
        if value.is_sign_positive() {
            scale
        } else {
            0.0
        }
    } else {
        value
    }
}

/// Alpha is a fraction, and a value outside it is brought back in rather than rejected
/// (css-color-4 §4.1): `lab(0 0 0 / 300%)` is opaque, not invalid.
fn clamp_alpha(alpha: f64) -> f64 {
    alpha.clamp(0.0, 1.0)
}

/// Alpha, which is a number from 0 to 1 or a percentage of it.
fn color_alpha(value: &CssValue) -> Option<Option<f64>> {
    color_component(value, 1.0)
}

/// Split a colour function's arguments into its components and its alpha.
///
/// Both notations are accepted: the legacy comma form, where the alpha is simply the fourth
/// item, and the modern form, where it follows a solidus. Mixing them is not a colour, and
/// neither is a comma form for a function that never had one.
fn split_color_args<'a>(name: &str, args: &'a [CssValue]) -> Option<(Vec<&'a CssValue>, Option<&'a CssValue>)> {
    let is = |value: &CssValue, text: &str| matches!(value, CssValue::String(word) if word == text);
    let commas = args.iter().any(|value| matches!(value, CssValue::Comma));
    let solidus = args.iter().position(|value| is(value, "/"));

    if commas {
        // `hwb()`, `lab()` and everything newer postdate the comma form and never had one.
        if !matches!(name, "rgb" | "rgba" | "hsl" | "hsla") || solidus.is_some() {
            return None;
        }
        // `none` postdates the comma form too, so `hsl(none, none, none)` is not a colour.
        if args
            .iter()
            .any(|value| matches!(value, CssValue::String(word) if word.eq_ignore_ascii_case("none")))
        {
            return None;
        }
        let mut items: Vec<&CssValue> = Vec::with_capacity(4);
        for value in args {
            if !matches!(value, CssValue::Comma) {
                items.push(value);
            }
        }
        // A comma form gives every component or none of them; `rgb(1, 2)` is not a colour.
        let alpha = if items.len() == 4 { items.pop() } else { None };
        if items.len() != 3 {
            return None;
        }
        return Some((items, alpha));
    }

    match solidus {
        Some(at) => {
            let alpha = args.get(at + 1)?;
            if args.len() != at + 2 {
                return None;
            }
            Some((args[..at].iter().collect(), Some(alpha)))
        }
        None => Some((args.iter().collect(), None)),
    }
}

/// Build a colour from one of the colour functions, keeping its components in that function's
/// own units. Returns `None` when the arguments are not a colour, which leaves the function
/// alone for a stage that knows more - or drops the declaration, if none does.
fn parse_css_color_function(name: &str, args: &[CssValue]) -> Option<CssColor> {
    fold_color_function(name, args, false)
}

/// Whether `name` is one of the colour functions and `args` make a colour of it.
///
/// `resolve_math` says which stage is asking. A `calc()` inside a colour component stays a
/// `calc()` in the *specified* value - `lab(calc(50 * 3) 0 0)` reads back from `element.style`
/// as `lab(calc(150) 0 0)`, not as `lab(100 0 0)` - so the parse refuses to fold such a colour
/// at all and leaves the function standing. The computed value is where the arithmetic is done.
pub(crate) fn fold_color_function(name: &str, args: &[CssValue], resolve_math: bool) -> Option<CssColor> {
    if !is_color_function(name) {
        return None;
    }
    let has_calc = args
        .iter()
        .any(|value| matches!(value, CssValue::Function(inner, _) if inner.eq_ignore_ascii_case("calc")));
    // A component that is itself a function has to reduce to a number before this can fold the
    // colour, and a `calc()` that came down to one term does. Anything still a function after
    // that - `var()`, `sibling-index()`, a `calc()` over one of them - means the colour is not
    // knowable here, so refuse rather than fold.
    let reduced: Vec<CssValue> = args.iter().map(reduce_color_component).collect();
    if reduced.iter().any(|v| matches!(v, CssValue::Function(..))) {
        return None;
    }
    let name = name.cow_to_ascii_lowercase();
    let name = name.as_ref();

    // `color()` names its space first, and its components are 0 to 1 within that space. It
    // always keeps its own notation, so a `calc()` inside it stays unresolved until computed.
    if name == "color" {
        if !resolve_math && has_calc {
            return None;
        }
        let space = match reduced.first() {
            Some(CssValue::String(word)) => PredefinedSpace::from_name(word)?,
            _ => return None,
        };
        let (components, alpha) = split_color_args(name, &reduced[1..])?;
        let [first, second, third] = components.as_slice() else {
            return None;
        };
        return Some(CssColor {
            syntax: ColorSyntax::Predefined(space),
            components: [
                color_component(first, 1.0)?,
                color_component(second, 1.0)?,
                color_component(third, 1.0)?,
            ],
            alpha: alpha.map_or(Some(Some(1.0)), color_alpha)?.map(clamp_alpha),
            computed: false,
        });
    }

    let (components, alpha) = split_color_args(name, &reduced)?;
    let [first, second, third] = components.as_slice() else {
        return None;
    };
    let alpha = alpha.map_or(Some(Some(1.0)), color_alpha)?;

    // Each notation reads its components in its own units: a percentage is a channel of 255 in
    // `rgb()`, a percentage of the axis in `lab()`, and simply itself in `hsl()`.
    let (syntax, components) = match name {
        "rgb" | "rgba" => (
            ColorSyntax::Rgb,
            [
                color_component(first, 255.0)?,
                color_component(second, 255.0)?,
                color_component(third, 255.0)?,
            ],
        ),
        "hsl" | "hsla" => (
            ColorSyntax::Hsl,
            [
                color_hue(first)?,
                color_component(second, 100.0)?,
                color_component(third, 100.0)?,
            ],
        ),
        "hwb" => (
            ColorSyntax::Hwb,
            [
                color_hue(first)?,
                color_component(second, 100.0)?,
                color_component(third, 100.0)?,
            ],
        ),
        "lab" => (
            ColorSyntax::Lab,
            [
                color_component(first, 100.0)?,
                color_component(second, 125.0)?,
                color_component(third, 125.0)?,
            ],
        ),
        "lch" => (
            ColorSyntax::Lch,
            [
                color_component(first, 100.0)?,
                color_component(second, 150.0)?,
                color_hue(third)?,
            ],
        ),
        "oklab" => (
            ColorSyntax::Oklab,
            [
                color_component(first, 1.0)?,
                color_component(second, 0.4)?,
                color_component(third, 0.4)?,
            ],
        ),
        "oklch" => (
            ColorSyntax::Oklch,
            [
                color_component(first, 1.0)?,
                color_component(second, 0.4)?,
                color_hue(third)?,
            ],
        ),
        _ => return None,
    };
    // Lightness has ends, and chroma has a floor. css-color-4 §11 brings a value outside them
    // back in rather than rejecting it, so `lab(400 0 10)` is simply the lightest lab there is.
    let mut components = components;
    match syntax {
        ColorSyntax::Lab | ColorSyntax::Lch => components[0] = components[0].map(|l| l.clamp(0.0, 100.0)),
        ColorSyntax::Oklab | ColorSyntax::Oklch => components[0] = components[0].map(|l| l.clamp(0.0, 1.0)),
        _ => {}
    }
    if matches!(syntax, ColorSyntax::Lch | ColorSyntax::Oklch) {
        components[1] = components[1].map(|chroma| chroma.max(0.0));
    }

    let color = CssColor {
        syntax,
        components,
        alpha: alpha.map(clamp_alpha),
        computed: false,
    };
    // A colour that goes out through the legacy sRGB triple has nowhere to show a `calc()`, so
    // the arithmetic is done whatever stage is asking. One that keeps its own notation does have
    // somewhere, and the specified value has to show it.
    if !resolve_math && has_calc && color.keeps_its_notation() {
        return None;
    }
    Some(color)
}

impl gosub_interface::css3::CssValue for CssValue {
    fn new_string(value: &str) -> Self {
        CssValue::String(value.to_string())
    }

    fn new_percentage(value: f32) -> Self {
        CssValue::Percentage(f64::from(value))
    }

    fn new_unit(value: f32, unit: String) -> Self {
        CssValue::Unit(f64::from(value), unit)
    }

    fn new_color(r: f32, g: f32, b: f32, a: f32) -> Self {
        CssValue::Color(RgbColor::new(r, g, b, a).into())
    }

    fn new_number(value: f32) -> Self {
        CssValue::Number(f64::from(value), NumberKind::Integer)
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
            Some(*percent as f32)
        } else {
            None
        }
    }

    fn as_unit(&self) -> Option<(f32, &str)> {
        if let CssValue::Unit(value, unit) = &self {
            Some((*value as f32, unit))
        } else {
            None
        }
    }

    fn as_color(&self) -> Option<(f32, f32, f32, f32)> {
        if let CssValue::Color(color) = &self {
            let color = color.to_rgb();
            Some((color.r, color.g, color.b, color.a))
        } else {
            None
        }
    }

    fn as_number(&self) -> Option<f32> {
        match self {
            CssValue::Number(num, _) => Some(*num as f32),
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

    #[test]
    fn a_colour_serializes_as_rgb_not_as_hex() {
        // `#rrggbbaa` was a debug rendering. Every CSS consumer - `getComputedStyle`, a
        // round-trip through `element.style` - is defined to see the legacy `rgb()` form.
        assert_eq!(
            CssValue::Color(RgbColor::from("#ff0000").into()).to_string(),
            "rgb(255, 0, 0)"
        );
        assert_eq!(
            CssValue::Color(RgbColor::from("red").into()).to_string(),
            "rgb(255, 0, 0)"
        );
        assert_eq!(
            CssValue::Color(RgbColor::new(0.0, 0.0, 0.0, 127.5).into()).to_string(),
            "rgba(0, 0, 0, 0.5)"
        );
        // An alpha that came from a hex byte is not a round number; three decimals is what
        // tells two of the 256 steps apart without printing f32 noise.
        assert_eq!(
            CssValue::Color(RgbColor::new(1.0, 2.0, 3.0, 128.0).into()).to_string(),
            "rgba(1, 2, 3, 0.502)"
        );
    }

    #[test]
    fn a_url_serializes_with_its_argument_quoted() {
        let url = |u: &str| CssValue::Function("url".to_string(), vec![CssValue::String(u.to_string())]);
        assert_eq!(url("a/b.png").to_string(), r#"url("a/b.png")"#);
        // The quote that delimits the string, and the backslash that escapes it, cannot appear
        // raw inside it.
        assert_eq!(url(r#"a"b"#).to_string(), r#"url("a\"b")"#);
        assert_eq!(url(r"a\b").to_string(), r#"url("a\\b")"#);
    }

    #[test]
    fn a_colour_function_with_an_unresolved_component_is_not_folded() {
        // The component filter drops what it does not recognise, so a `calc()` it cannot
        // evaluate used to vanish and leave a fully opaque colour nobody wrote. The colour is
        // not knowable at parse time, so the function has to survive instead.
        let unresolved = CssValue::Function(
            "calc".to_string(),
            vec![
                CssValue::Number(0.1, NumberKind::Integer),
                CssValue::String("*".to_string()),
                CssValue::Function("sibling-index".to_string(), vec![]),
            ],
        );
        let args = vec![
            CssValue::Number(0.5, NumberKind::Integer),
            CssValue::Number(0.2, NumberKind::Integer),
            CssValue::Number(180.0, NumberKind::Integer),
            unresolved,
        ];
        assert_eq!(parse_css_color_function("oklch", &args), None);

        // A `calc()` that does come down to a number is folded straight away when the colour
        // serializes through the legacy sRGB triple, because that form has nowhere to show the
        // arithmetic anyway.
        let resolvable = CssValue::Function(
            "calc".to_string(),
            vec![
                CssValue::Number(100.0, NumberKind::Integer),
                CssValue::String("+".to_string()),
                CssValue::Number(55.0, NumberKind::Integer),
            ],
        );
        let args = vec![
            resolvable,
            CssValue::Number(0.0, NumberKind::Integer),
            CssValue::Number(0.0, NumberKind::Integer),
        ];
        assert_eq!(
            parse_css_color_function("rgb", &args).map(|color| color.to_rgb()),
            Some(RgbColor::new(155.0, 0.0, 0.0, 255.0))
        );

        // `lab()` keeps the notation it was written in, so it can show the `calc()` - and the
        // specified value has to. Only the computed stage does the sum.
        assert_eq!(parse_css_color_function("lab", &args), None);
        assert_eq!(
            fold_color_function("lab", &args, true).map(|color| color.components[0]),
            Some(Some(100.0))
        );
    }

    #[test]
    fn a_function_does_not_double_its_comma_separators() {
        // The argument list carries its own `,` as a `CssValue::Comma`, so joining with ", "
        // as well wrote both: `min(50%, 100px)` came back as `min(50%, ,, 100px)`. Anything
        // reading a computed value as text got that.
        let value = CssValue::Function(
            "min".to_string(),
            vec![
                CssValue::Percentage(50.0),
                CssValue::Comma,
                CssValue::Unit(100.0, "px".to_string()),
            ],
        );
        assert_eq!(value.to_string(), "min(50%, 100px)");
    }

    #[test]
    fn space_separated_arguments_keep_their_space() {
        // `translate(1px 2px)` has no comma at all; the arguments must not run together.
        let value = CssValue::Function(
            "translate".to_string(),
            vec![
                CssValue::Unit(1.0, "px".to_string()),
                CssValue::Unit(2.0, "px".to_string()),
            ],
        );
        assert_eq!(value.to_string(), "translate(1px 2px)");
    }

    #[test]
    fn a_list_serializes_as_css_rather_than_as_a_debug_wrapper() {
        // `margin: 1px 2px` is held as a list. `List(1px, 2px)` was a debug rendering that
        // reached every consumer reading a computed value as text.
        let value = CssValue::List(vec![
            CssValue::Unit(1.0, "px".to_string()),
            CssValue::Unit(2.0, "px".to_string()),
        ]);
        assert_eq!(value.to_string(), "1px 2px");

        let commas = CssValue::List(vec![
            CssValue::String("a".to_string()),
            CssValue::Comma,
            CssValue::String("b".to_string()),
        ]);
        assert_eq!(commas.to_string(), "a, b");
    }

    /// A `calc()` built with a raw text body, as only a test does, has no `CssValue::Unit` in it.
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
        let rule = CssRule::new(
            vec![CssSelector::new(vec![vec![CssSelectorPart::Type("h1".to_string())]])],
            vec![CssDeclaration {
                property: "color".to_string(),
                value: CssValue::String("red".to_string()),
                important: false,
            }],
            None,
            None,
        );

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
        let selector = CssSelector::new(vec![vec![
            CssSelectorPart::Type("h1".to_string()),
            CssSelectorPart::Class("myclass".to_string()),
            CssSelectorPart::Id("myid".to_string()),
        ]]);

        let specificity = selector.specificity();
        assert_eq!(specificity, [Specificity::new(1, 1, 1)]);

        let selector = CssSelector::new(vec![vec![
            CssSelectorPart::Type("h1".to_string()),
            CssSelectorPart::Class("myclass".to_string()),
        ]]);

        let specificity = selector.specificity();
        assert_eq!(specificity, [Specificity::new(0, 1, 1)]);

        let selector = CssSelector::new(vec![vec![CssSelectorPart::Type("h1".to_string())]]);

        let specificity = selector.specificity();
        assert_eq!(specificity, [Specificity::new(0, 0, 1)]);

        let selector = CssSelector::new(vec![vec![
            CssSelectorPart::Class("myclass".to_string()),
            CssSelectorPart::Class("otherclass".to_string()),
        ]]);

        let specificity = selector.specificity();
        assert_eq!(specificity, [Specificity::new(0, 2, 0)]);
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
                CssValue::Number(14.0, NumberKind::Integer),
                CssValue::Comma,
                CssValue::Number(42.0, NumberKind::Integer),
                CssValue::Comma,
                CssValue::Number(54.0, NumberKind::Integer),
                CssValue::Comma,
                CssValue::Number(0.5, NumberKind::Integer),
            ],
        )
        .expect("rgba should parse");
        let c = c.to_rgb();
        assert_eq!((c.r, c.g, c.b), (14.0, 42.0, 54.0));
        assert!((c.a - 127.5).abs() < 0.5);

        // rgb() without alpha is fully opaque.
        let c = parse_css_color_function(
            "rgb",
            &[
                CssValue::Number(255.0, NumberKind::Integer),
                CssValue::Number(0.0, NumberKind::Integer),
                CssValue::Number(0.0, NumberKind::Integer),
            ],
        )
        .unwrap();
        let c = c.to_rgb();
        assert_eq!((c.r, c.g, c.b, c.a), (255.0, 0.0, 0.0, 255.0));

        // hsl(0 100% 50%) == red.
        let c = parse_css_color_function(
            "hsl",
            &[
                CssValue::Number(0.0, NumberKind::Integer),
                CssValue::Percentage(100.0),
                CssValue::Percentage(50.0),
            ],
        )
        .unwrap();
        let c = c.to_rgb();
        assert!((c.r - 255.0).abs() < 1.0 && c.g < 1.0 && c.b < 1.0, "hsl red got {c:?}");

        // A color function collapses to CssValue::Color at AST conversion time.
        assert!(is_color_function("rgba") && !is_color_function("calc"));
    }
}
