//! Media query representation and evaluation.
//!
//! The parser already understands `@media` preludes fully ([`crate::parser::at_rule::media`]);
//! this module turns that AST into a form the cascade can test cheaply, and evaluates it
//! against a [`MediaEnvironment`].
//!
//! Queries are attached to [`crate::stylesheet::CssRule`]s rather than resolved when the
//! stylesheet is built, so a viewport change only needs a restyle - never a re-parse.

use std::cell::Cell;

use crate::node::{Node, NodeType};
use cow_utils::CowUtils;

/// CSS px per CSS inch - fixed by the spec, independent of the physical display.
const PX_PER_IN: f32 = 96.0;
/// CSS px per CSS cm (`96 / 2.54`).
const PX_PER_CM: f32 = PX_PER_IN / 2.54;
/// Font size that `em`/`rem` resolve against inside a media query. Per spec these use the
/// *initial* `font-size`, not the value on any element, so no element context is needed.
const MEDIA_QUERY_FONT_SIZE: f32 = 16.0;

/// The `media_type` a query can name. Types outside this set are parsed but never match,
/// which is what the spec asks for ("unknown media type" is false).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MediaType {
    /// A screen-like device. What the engine reports while rendering to a window or tile.
    #[default]
    Screen,
    /// Paged output.
    Print,
}

/// Value of the `prefers-color-scheme` feature.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColorScheme {
    #[default]
    Light,
    Dark,
}

/// Value of the `prefers-reduced-motion` feature.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ReducedMotion {
    #[default]
    NoPreference,
    Reduce,
}

/// Everything a media query can ask about the output device and user preferences.
///
/// Held per-thread (see [`set_media_environment`]) because style computation runs on the tab's
/// own worker thread and the `CssSystem` trait carries no environment parameter. All fields are
/// `Copy` so the thread-local can be a plain [`Cell`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MediaEnvironment {
    /// Layout viewport width in CSS px. Also what `vw` resolves against.
    pub width: f32,
    /// Layout viewport height in CSS px. Also what `vh` resolves against.
    pub height: f32,
    /// Screen width in CSS px, for the legacy `device-width` family.
    pub device_width: f32,
    /// Screen height in CSS px, for the legacy `device-height` family.
    pub device_height: f32,
    /// Device pixel ratio, reported to `resolution` (as `dppx`) and
    /// `-webkit-device-pixel-ratio`.
    pub device_pixel_ratio: f32,
    pub media_type: MediaType,
    pub color_scheme: ColorScheme,
    pub reduced_motion: ReducedMotion,
    /// Whether scripting is enabled, for the `scripting` feature. False until the JS runtime
    /// is wired into the engine.
    pub scripting: bool,
}

impl Default for MediaEnvironment {
    fn default() -> Self {
        Self {
            width: 1280.0,
            height: 800.0,
            device_width: 1280.0,
            device_height: 800.0,
            device_pixel_ratio: 1.0,
            media_type: MediaType::Screen,
            color_scheme: ColorScheme::Light,
            reduced_motion: ReducedMotion::NoPreference,
            scripting: false,
        }
    }
}

thread_local! {
    /// The environment media queries and viewport-relative units resolve against on this
    /// thread. Defaults to a 1280x800 light screen so styles still resolve sensibly before
    /// any real viewport is known.
    static MEDIA_ENVIRONMENT: Cell<MediaEnvironment> = const {
        Cell::new(MediaEnvironment {
            width: 1280.0,
            height: 800.0,
            device_width: 1280.0,
            device_height: 800.0,
            device_pixel_ratio: 1.0,
            media_type: MediaType::Screen,
            color_scheme: ColorScheme::Light,
            reduced_motion: ReducedMotion::NoPreference,
            scripting: false,
        })
    };
}

/// Install the environment used by subsequent style computations on this thread. The render
/// flow calls this once per pass, before the render tree is built.
pub fn set_media_environment(env: MediaEnvironment) {
    MEDIA_ENVIRONMENT.with(|cell| cell.set(env));
}

/// The environment in force on this thread.
#[must_use]
pub fn media_environment() -> MediaEnvironment {
    MEDIA_ENVIRONMENT.with(Cell::get)
}

/// A comma-separated list of media queries: it matches when *any* query does.
///
/// An empty list matches everything, which keeps `@media { ... }` with an unparseable prelude
/// from silently hiding its rules.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MediaQueryList {
    pub queries: Vec<MediaQuery>,
}

impl MediaQueryList {
    /// Build a list from a [`NodeType::MediaQueryList`] prelude. Any other node yields an
    /// always-matching (empty) list.
    #[must_use]
    pub fn from_ast(node: &Node) -> Self {
        let NodeType::MediaQueryList { media_queries } = &node.node_type else {
            return Self::default();
        };
        Self {
            queries: media_queries.iter().filter_map(MediaQuery::from_ast).collect(),
        }
    }

    #[must_use]
    pub fn matches(&self, env: &MediaEnvironment) -> bool {
        self.queries.is_empty() || self.queries.iter().any(|query| query.matches(env))
    }
}

/// A single media query: an optional type (`screen`), an optional condition
/// (`(min-width: 40em) and (hover)`), and an optional leading `not`.
#[derive(Debug, Clone, PartialEq)]
pub struct MediaQuery {
    /// `not` inverts the query as a whole, per spec - not just the condition.
    pub negated: bool,
    /// `None` when the query names no type, or names `all` - either matches any device.
    /// An unrecognised type is recorded in `unknown_type` instead.
    pub media_type: Option<MediaType>,
    /// True when a media type was given that this engine does not recognise. Such a query
    /// never matches on its own, but `not <unknown>` still matches.
    pub unknown_type: bool,
    pub condition: Option<MediaCondition>,
}

impl MediaQuery {
    fn from_ast(node: &Node) -> Option<Self> {
        let NodeType::MediaQuery {
            modifier,
            media_type,
            condition,
        } = &node.node_type
        else {
            return None;
        };

        // `only` is a legacy shield against CSS2 user agents and has no effect here.
        let negated = modifier.cow_to_lowercase() == "not";

        let (media_type, unknown_type) = match media_type.cow_to_lowercase().as_ref() {
            "" | "all" => (None, false),
            "screen" => (Some(MediaType::Screen), false),
            "print" => (Some(MediaType::Print), false),
            _ => (None, true),
        };

        Some(Self {
            negated,
            media_type,
            unknown_type,
            condition: condition.as_deref().and_then(MediaCondition::from_ast),
        })
    }

    #[must_use]
    pub fn matches(&self, env: &MediaEnvironment) -> bool {
        let matched = !self.unknown_type
            && self.media_type.is_none_or(|ty| ty == env.media_type)
            && self.condition.as_ref().is_none_or(|cond| cond.matches(env));

        matched != self.negated
    }
}

/// A boolean combination of media features.
///
/// The parser hands back a flat list of terms and `and`/`or`/`not` keywords
/// ([`NodeType::Condition`]); this is the tree recovered from it. Because the spec forbids
/// mixing `and` and `or` at one level without parentheses, `or` of `and`-groups covers
/// everything that can legally appear.
#[derive(Debug, Clone, PartialEq)]
pub enum MediaCondition {
    /// A plain feature test: `(hover)`, `(min-width: 40em)`.
    Feature(MediaFeature),
    /// Range syntax: `(width > 400px)`, `(400px <= width <= 700px)`.
    Range(MediaRange),
    Not(Box<MediaCondition>),
    All(Vec<MediaCondition>),
    Any(Vec<MediaCondition>),
    /// A term this engine cannot interpret. Evaluates to false, so `not (weird)` is true -
    /// which is what the spec's "unknown features are false" rule implies.
    Unknown,
}

impl MediaCondition {
    fn from_ast(node: &Node) -> Option<Self> {
        let NodeType::Condition { list } = &node.node_type else {
            return Self::term_from_ast(node);
        };

        let mut or_groups: Vec<Vec<MediaCondition>> = Vec::new();
        let mut current: Vec<MediaCondition> = Vec::new();
        let mut pending_not = false;

        for item in list {
            if let NodeType::Ident { value } = &item.node_type {
                match value.cow_to_lowercase().as_ref() {
                    "not" => pending_not = true,
                    // `and` merely separates terms in the group being built.
                    "and" => {}
                    "or" => or_groups.push(std::mem::take(&mut current)),
                    // A bare identifier that is not an operator is not a valid term.
                    _ => current.push(MediaCondition::Unknown),
                }
                continue;
            }

            let term = Self::term_from_ast(item).unwrap_or(MediaCondition::Unknown);
            current.push(if std::mem::take(&mut pending_not) {
                MediaCondition::Not(Box::new(term))
            } else {
                term
            });
        }
        or_groups.push(current);

        let mut groups: Vec<MediaCondition> = or_groups
            .into_iter()
            .filter(|group| !group.is_empty())
            .map(|mut group| {
                if group.len() == 1 {
                    // `swap_remove` avoids cloning the sole term out of the vec.
                    group.swap_remove(0)
                } else {
                    MediaCondition::All(group)
                }
            })
            .collect();

        match groups.len() {
            0 => None,
            1 => Some(groups.swap_remove(0)),
            _ => Some(MediaCondition::Any(groups)),
        }
    }

    /// Convert a single non-operator node (a feature or a range) into a condition.
    fn term_from_ast(node: &Node) -> Option<Self> {
        match &node.node_type {
            NodeType::Feature { name, value, .. } => Some(MediaCondition::Feature(MediaFeature {
                name: name.cow_to_lowercase().into_owned(),
                value: value.as_deref().and_then(FeatureValue::from_ast),
            })),
            NodeType::Range {
                left,
                left_comparison,
                middle,
                right_comparison,
                right,
            } => MediaRange::from_ast(
                left,
                left_comparison,
                middle,
                right_comparison.as_deref(),
                right.as_deref(),
            )
            .map(MediaCondition::Range),
            _ => None,
        }
    }

    #[must_use]
    pub fn matches(&self, env: &MediaEnvironment) -> bool {
        match self {
            MediaCondition::Feature(feature) => feature.matches(env),
            MediaCondition::Range(range) => range.matches(env),
            MediaCondition::Not(inner) => !inner.matches(env),
            MediaCondition::All(list) => list.iter().all(|cond| cond.matches(env)),
            MediaCondition::Any(list) => list.iter().any(|cond| cond.matches(env)),
            MediaCondition::Unknown => false,
        }
    }
}

/// A value on the right-hand side of a media feature, normalised to the unit the comparison
/// needs (CSS px for lengths, `dppx` for resolutions).
#[derive(Debug, Clone, PartialEq)]
pub enum FeatureValue {
    Number(f32),
    Length(f32),
    Resolution(f32),
    Ratio(f32),
    Ident(String),
}

impl FeatureValue {
    fn from_ast(node: &Node) -> Option<Self> {
        match &node.node_type {
            NodeType::Number { value } => Some(FeatureValue::Number(*value)),
            NodeType::Ident { value } => Some(FeatureValue::Ident(value.cow_to_lowercase().into_owned())),
            NodeType::Dimension { value, unit } => Self::from_dimension(*value, unit),
            // A ratio arrives as `<number> / <number>`.
            NodeType::Value { children } => match children.as_slice() {
                [num, op, den] if matches!(&op.node_type, NodeType::Operator(o) if o == "/") => {
                    let num = Self::from_ast(num)?.as_number()?;
                    let den = Self::from_ast(den)?.as_number()?;
                    (den != 0.0).then_some(FeatureValue::Ratio(num / den))
                }
                _ => None,
            },
            _ => None,
        }
    }

    fn from_dimension(value: f32, unit: &str) -> Option<Self> {
        Some(match unit.cow_to_lowercase().as_ref() {
            "px" => FeatureValue::Length(value),
            "em" | "rem" => FeatureValue::Length(value * MEDIA_QUERY_FONT_SIZE),
            "pt" => FeatureValue::Length(value * PX_PER_IN / 72.0),
            "pc" => FeatureValue::Length(value * PX_PER_IN / 6.0),
            "in" => FeatureValue::Length(value * PX_PER_IN),
            "cm" => FeatureValue::Length(value * PX_PER_CM),
            "mm" => FeatureValue::Length(value * PX_PER_CM / 10.0),
            "q" => FeatureValue::Length(value * PX_PER_CM / 40.0),
            "dppx" | "x" => FeatureValue::Resolution(value),
            "dpi" => FeatureValue::Resolution(value / PX_PER_IN),
            "dpcm" => FeatureValue::Resolution(value / PX_PER_CM),
            _ => return None,
        })
    }

    /// The value as a plain number, for features that compare numerically. Lengths and
    /// resolutions are already in the canonical unit, so they pass through.
    fn as_number(&self) -> Option<f32> {
        match self {
            FeatureValue::Number(v)
            | FeatureValue::Length(v)
            | FeatureValue::Resolution(v)
            | FeatureValue::Ratio(v) => Some(*v),
            FeatureValue::Ident(_) => None,
        }
    }

    fn as_ident(&self) -> Option<&str> {
        match self {
            FeatureValue::Ident(value) => Some(value.as_str()),
            _ => None,
        }
    }
}

/// The `min-`/`max-` prefix on a feature name, which the spec defines as `>=` / `<=`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Bound {
    Min,
    Max,
    Exact,
}

/// Split a feature name into its range bound and its bare name.
///
/// A `-webkit-` prefix is dropped first, because WebKit puts the bound *inside* it:
/// `-webkit-min-device-pixel-ratio` is `min-` applied to `device-pixel-ratio`. Sites still
/// ship that form alongside standard `resolution`, so it is worth understanding.
fn split_bound(name: &str) -> (Bound, &str) {
    let name = name.strip_prefix("-webkit-").unwrap_or(name);
    if let Some(rest) = name.strip_prefix("min-") {
        (Bound::Min, rest)
    } else if let Some(rest) = name.strip_prefix("max-") {
        (Bound::Max, rest)
    } else {
        (Bound::Exact, name)
    }
}

/// A single `(name)` or `(name: value)` test.
#[derive(Debug, Clone, PartialEq)]
pub struct MediaFeature {
    /// Lowercased feature name, `min-`/`max-` prefix included.
    pub name: String,
    /// `None` for the boolean form, `(hover)`.
    pub value: Option<FeatureValue>,
}

impl MediaFeature {
    #[must_use]
    pub fn matches(&self, env: &MediaEnvironment) -> bool {
        let (bound, name) = split_bound(&self.name);

        match &self.value {
            Some(value) => evaluate_feature(name, bound, value, env),
            // Boolean form: true when the feature's value is neither zero nor `none`.
            None => evaluate_boolean_feature(name, env),
        }
    }
}

/// A range test - `(width > 400px)` or `(400px <= width <= 700px)`.
#[derive(Debug, Clone, PartialEq)]
pub struct MediaRange {
    /// Lowercased name of the feature under test.
    pub name: String,
    /// Comparisons the feature's value must satisfy, as `(operator, value)` with the feature
    /// on the left: `400px < width` is stored as `(">", 400px)`.
    comparisons: Vec<(Comparison, FeatureValue)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Comparison {
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
}

impl Comparison {
    fn from_ast(node: &Node) -> Option<Self> {
        let NodeType::Operator(op) = &node.node_type else {
            return None;
        };
        Some(match op.as_str() {
            "<" => Comparison::Lt,
            "<=" => Comparison::Le,
            ">" => Comparison::Gt,
            ">=" => Comparison::Ge,
            "=" => Comparison::Eq,
            _ => return None,
        })
    }

    /// The same comparison read right-to-left, so `400px < width` becomes `width > 400px`.
    fn flipped(self) -> Self {
        match self {
            Comparison::Lt => Comparison::Gt,
            Comparison::Le => Comparison::Ge,
            Comparison::Gt => Comparison::Lt,
            Comparison::Ge => Comparison::Le,
            Comparison::Eq => Comparison::Eq,
        }
    }

    fn test(self, lhs: f32, rhs: f32) -> bool {
        match self {
            Comparison::Lt => lhs < rhs,
            Comparison::Le => lhs <= rhs,
            Comparison::Gt => lhs > rhs,
            Comparison::Ge => lhs >= rhs,
            Comparison::Eq => (lhs - rhs).abs() < f32::EPSILON,
        }
    }
}

impl MediaRange {
    fn from_ast(
        left: &Node,
        left_comparison: &Node,
        middle: &Node,
        right_comparison: Option<&Node>,
        right: Option<&Node>,
    ) -> Option<Self> {
        let left_op = Comparison::from_ast(left_comparison)?;

        // The feature name is whichever side is a bare identifier. `width > 400px` puts it on
        // the left; `400px < width < 700px` puts it in the middle.
        if let NodeType::Ident { value } = &left.node_type {
            // `<name> <op> <value>` - a second comparison is not legal in this shape.
            return Some(Self {
                name: value.cow_to_lowercase().into_owned(),
                comparisons: vec![(left_op, FeatureValue::from_ast(middle)?)],
            });
        }

        let NodeType::Ident { value: name } = &middle.node_type else {
            return None;
        };
        // `<value> <op> <name>` reads backwards, so flip it to put the feature on the left.
        let mut comparisons = vec![(left_op.flipped(), FeatureValue::from_ast(left)?)];
        if let (Some(op), Some(value)) = (right_comparison, right) {
            comparisons.push((Comparison::from_ast(op)?, FeatureValue::from_ast(value)?));
        }

        Some(Self {
            name: name.cow_to_lowercase().into_owned(),
            comparisons,
        })
    }

    #[must_use]
    pub fn matches(&self, env: &MediaEnvironment) -> bool {
        // Range syntax carries no `min-`/`max-` prefix, but a `-webkit-` one is still possible.
        let Some(actual) = numeric_feature_value(split_bound(&self.name).1, env) else {
            return false;
        };
        self.comparisons
            .iter()
            .all(|(op, value)| value.as_number().is_some_and(|expected| op.test(actual, expected)))
    }
}

/// The engine's value for a feature that compares numerically, in its canonical unit
/// (CSS px for lengths, `dppx` for resolutions). `None` for unknown or keyword-only features.
fn numeric_feature_value(name: &str, env: &MediaEnvironment) -> Option<f32> {
    Some(match name {
        "width" => env.width,
        "height" => env.height,
        "device-width" => env.device_width,
        "device-height" => env.device_height,
        "aspect-ratio" if env.height != 0.0 => env.width / env.height,
        "device-aspect-ratio" if env.device_height != 0.0 => env.device_width / env.device_height,
        // `resolution` is in dppx, which is exactly what the device pixel ratio means.
        // `device-pixel-ratio` is the `-webkit-` name, already stripped by `split_bound`.
        "resolution" | "device-pixel-ratio" => env.device_pixel_ratio,
        // Colour depth per component. Gosub always composites 8-bit sRGB.
        "color" => 8.0,
        "color-index" => 0.0,
        "monochrome" => 0.0,
        // A non-zero value here means "grid-based device" (a terminal), which we are not.
        "grid" => 0.0,
        _ => return None,
    })
}

/// Evaluate `(name: value)`, honouring a `min-`/`max-` prefix.
fn evaluate_feature(name: &str, bound: Bound, value: &FeatureValue, env: &MediaEnvironment) -> bool {
    if let Some(actual) = numeric_feature_value(name, env) {
        let Some(expected) = value.as_number() else {
            return false;
        };
        return match bound {
            Bound::Min => actual >= expected,
            Bound::Max => actual <= expected,
            Bound::Exact => (actual - expected).abs() < f32::EPSILON,
        };
    }

    // Keyword features. A `min-`/`max-` prefix is meaningless on these, so they only answer
    // to the exact form.
    if bound != Bound::Exact {
        return false;
    }
    let Some(keyword) = value.as_ident() else {
        return false;
    };
    match name {
        "orientation" => match keyword {
            // Square viewports are portrait, per spec.
            "portrait" => env.height >= env.width,
            "landscape" => env.width > env.height,
            _ => false,
        },
        "prefers-color-scheme" => match keyword {
            "light" => env.color_scheme == ColorScheme::Light,
            "dark" => env.color_scheme == ColorScheme::Dark,
            _ => false,
        },
        "prefers-reduced-motion" => match keyword {
            "no-preference" => env.reduced_motion == ReducedMotion::NoPreference,
            "reduce" => env.reduced_motion == ReducedMotion::Reduce,
            _ => false,
        },
        "scripting" => match keyword {
            "enabled" => env.scripting,
            "none" => !env.scripting,
            // We never run script only during the initial page load.
            "initial-only" => false,
            _ => false,
        },
        // Gosub renders to a pointer-driven window with a real mouse.
        "hover" | "any-hover" => keyword == "hover",
        "pointer" | "any-pointer" => keyword == "fine",
        "prefers-contrast" => keyword == "no-preference",
        "forced-colors" => keyword == "none",
        "display-mode" => keyword == "browser",
        "update" => keyword == "fast",
        "overflow-block" | "overflow-inline" => keyword == "scroll",
        // Unknown feature: false, so that `not (unknown)` is true.
        _ => false,
    }
}

/// Evaluate the boolean form `(name)`: true when the feature's value is neither zero nor the
/// "none" keyword.
fn evaluate_boolean_feature(name: &str, env: &MediaEnvironment) -> bool {
    if let Some(actual) = numeric_feature_value(name, env) {
        return actual != 0.0;
    }
    match name {
        "orientation" | "prefers-color-scheme" | "display-mode" | "update" | "overflow-block" | "overflow-inline" => {
            true
        }
        "hover" | "any-hover" | "pointer" | "any-pointer" => true,
        "scripting" => env.scripting,
        // `prefers-reduced-motion`, `prefers-contrast` and `forced-colors` are all false in
        // their "no-preference"/"none" state, which is what we report.
        "prefers-reduced-motion" => env.reduced_motion != ReducedMotion::NoPreference,
        "prefers-contrast" | "forced-colors" => false,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Css3;
    use gosub_interface::css3::CssOrigin;
    use gosub_shared::config::ParserConfig;

    /// Parse `@media <query> { a { color: red } }` and return the query list attached to the
    /// single rule it contains.
    fn query_list(query: &str) -> MediaQueryList {
        let css = format!("@media {query} {{ a {{ color: red; }} }}");
        let sheet = Css3::parse_str(&css, ParserConfig::default(), CssOrigin::Author, "test.css")
            .expect("media prelude should parse");
        assert_eq!(sheet.rules.len(), 1, "the inner rule should be collected: {css}");
        let media = sheet.rules[0]
            .media
            .as_ref()
            .expect("the rule should carry its media conditions");
        assert_eq!(media.len(), 1, "one @media level");
        (*media[0]).clone()
    }

    fn env(width: f32, height: f32) -> MediaEnvironment {
        MediaEnvironment {
            width,
            height,
            device_width: width,
            device_height: height,
            ..MediaEnvironment::default()
        }
    }

    fn matches(query: &str, env: &MediaEnvironment) -> bool {
        query_list(query).matches(env)
    }

    #[test]
    fn min_and_max_width() {
        let narrow = env(500.0, 800.0);
        let wide = env(1200.0, 800.0);

        assert!(!matches("(min-width: 768px)", &narrow));
        assert!(matches("(min-width: 768px)", &wide));
        assert!(matches("(max-width: 768px)", &narrow));
        assert!(!matches("(max-width: 768px)", &wide));
    }

    #[test]
    fn bounds_are_inclusive() {
        let exact = env(768.0, 800.0);
        assert!(matches("(min-width: 768px)", &exact));
        assert!(matches("(max-width: 768px)", &exact));
    }

    #[test]
    fn em_units_use_the_initial_font_size() {
        // 40em == 640px, so a 700px viewport is over the breakpoint and a 600px one is under.
        assert!(matches("(min-width: 40em)", &env(700.0, 800.0)));
        assert!(!matches("(min-width: 40em)", &env(600.0, 800.0)));
    }

    #[test]
    fn media_type_gates_the_query() {
        let wide = env(1200.0, 800.0);
        assert!(matches("screen", &wide));
        assert!(!matches("print", &wide));
        assert!(matches("screen and (min-width: 768px)", &wide));
        assert!(!matches("print and (min-width: 768px)", &wide));
        assert!(matches("all and (min-width: 768px)", &wide));
    }

    #[test]
    fn only_modifier_is_transparent() {
        assert!(matches("only screen and (min-width: 768px)", &env(1200.0, 800.0)));
    }

    #[test]
    fn not_inverts_the_whole_query() {
        let wide = env(1200.0, 800.0);
        // The type matches and the condition matches, so `not` makes the query false.
        assert!(!matches("not screen and (min-width: 768px)", &wide));
        // The condition fails, so the negated query is true.
        assert!(matches("not screen and (min-width: 2000px)", &wide));
        assert!(matches("not print", &wide));
    }

    #[test]
    fn comma_is_or() {
        let narrow = env(400.0, 800.0);
        assert!(matches("(max-width: 500px), (min-width: 1000px)", &narrow));
        assert!(!matches("(min-width: 800px), (min-width: 1000px)", &narrow));
    }

    #[test]
    fn and_requires_every_term() {
        let mid = env(800.0, 600.0);
        assert!(matches("(min-width: 700px) and (max-width: 900px)", &mid));
        assert!(!matches("(min-width: 700px) and (max-width: 750px)", &mid));
    }

    #[test]
    fn range_syntax() {
        let mid = env(800.0, 600.0);
        assert!(matches("(width > 400px)", &mid));
        assert!(!matches("(width > 900px)", &mid));
        assert!(matches("(width >= 800px)", &mid));
        assert!(matches("(400px < width)", &mid));
        assert!(!matches("(900px < width)", &mid));
    }

    #[test]
    fn double_ended_range() {
        assert!(matches("(400px <= width <= 700px)", &env(500.0, 600.0)));
        assert!(!matches("(400px <= width <= 700px)", &env(800.0, 600.0)));
        assert!(!matches("(400px <= width <= 700px)", &env(300.0, 600.0)));
    }

    #[test]
    fn orientation() {
        assert!(matches("(orientation: landscape)", &env(1200.0, 800.0)));
        assert!(!matches("(orientation: portrait)", &env(1200.0, 800.0)));
        assert!(matches("(orientation: portrait)", &env(600.0, 900.0)));
        // A square viewport counts as portrait.
        assert!(matches("(orientation: portrait)", &env(800.0, 800.0)));
    }

    #[test]
    fn aspect_ratio() {
        let wide = env(1600.0, 900.0);
        assert!(matches("(min-aspect-ratio: 16/9)", &wide));
        assert!(matches("(max-aspect-ratio: 16/9)", &wide));
        assert!(!matches("(min-aspect-ratio: 2/1)", &wide));
    }

    #[test]
    fn prefers_color_scheme() {
        let light = MediaEnvironment::default();
        let dark = MediaEnvironment {
            color_scheme: ColorScheme::Dark,
            ..MediaEnvironment::default()
        };
        assert!(matches("(prefers-color-scheme: light)", &light));
        assert!(!matches("(prefers-color-scheme: dark)", &light));
        assert!(matches("(prefers-color-scheme: dark)", &dark));
    }

    #[test]
    fn resolution_reads_the_device_pixel_ratio() {
        let hidpi = MediaEnvironment {
            device_pixel_ratio: 2.0,
            ..MediaEnvironment::default()
        };
        assert!(matches("(min-resolution: 2dppx)", &hidpi));
        assert!(matches("(min-resolution: 192dpi)", &hidpi));
        assert!(!matches("(min-resolution: 3dppx)", &hidpi));
        assert!(!matches("(min-resolution: 2dppx)", &MediaEnvironment::default()));
    }

    #[test]
    fn webkit_device_pixel_ratio() {
        let hidpi = MediaEnvironment {
            device_pixel_ratio: 2.0,
            ..MediaEnvironment::default()
        };
        // WebKit puts the bound inside the vendor prefix.
        assert!(matches("(-webkit-min-device-pixel-ratio: 2)", &hidpi));
        assert!(!matches("(-webkit-min-device-pixel-ratio: 3)", &hidpi));
        assert!(!matches(
            "(-webkit-min-device-pixel-ratio: 2)",
            &MediaEnvironment::default()
        ));
    }

    #[test]
    fn boolean_form() {
        assert!(matches("(hover)", &MediaEnvironment::default()));
        assert!(matches("(color)", &MediaEnvironment::default()));
        assert!(!matches("(monochrome)", &MediaEnvironment::default()));
        assert!(!matches("(scripting)", &MediaEnvironment::default()));
    }

    #[test]
    fn unknown_features_are_false_but_negate_to_true() {
        let default = MediaEnvironment::default();
        assert!(!matches("(totally-made-up: 3px)", &default));
        assert!(matches("not all and (totally-made-up: 3px)", &default));
    }

    #[test]
    fn unknown_media_type_never_matches() {
        let default = MediaEnvironment::default();
        assert!(!matches("aural", &default));
        assert!(matches("not aural", &default));
    }
}
