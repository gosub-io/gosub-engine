//! What the engine could not use from a page: the HTML parser's errors and the CSS it dropped.
//!
//! Gathered once per navigation, after the first frame, when styles have been computed and the
//! cascade has checked the values of every rule the page uses. It travels as
//! [`NavigationEvent::Diagnostics`](crate::engine::events::NavigationEvent::Diagnostics), and
//! from a renderer process inside its [`PageSummary`](crate::fork_server::protocol::PageSummary).

use crate::html::{EngineDocument, RenderConfiguration};
use gosub_interface::css3::{CssDroppedKind, CssOrigin, CssStylesheet};
use gosub_interface::document::Document;
use serde::{Deserialize, Serialize};

/// The most HTML parse errors a report carries. A page of broken markup can have thousands,
/// and the report crosses from a renderer process; the count says how many there were.
pub const MAX_HTML_ERRORS: usize = 500;

/// The most dropped CSS entries a report carries, across all of a page's stylesheets.
pub const MAX_CSS_DROPPED: usize = 500;

/// One navigation's HTML parse errors and dropped CSS.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PageDiagnostics {
    /// The HTML parser's errors, in the order it met them, up to [`MAX_HTML_ERRORS`].
    pub html_errors: Vec<HtmlParseError>,
    /// How many HTML parse errors there were in all.
    pub html_error_count: usize,
    /// What the page's own stylesheets said that the engine did not apply, up to
    /// [`MAX_CSS_DROPPED`]. A value is checked when an element first uses its rule, so one in a
    /// rule nothing matched is not here; everything the parse rejects is.
    pub css_dropped: Vec<DroppedCss>,
    /// How many dropped CSS entries there were in all. A stylesheet reports at most 512 of its
    /// own, so a sheet past that is counted as 512.
    pub css_dropped_count: usize,
}

/// An error the HTML parser reported.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HtmlParseError {
    /// What the parser found: the spec's name for a tokenizer error, a description for a
    /// tree-construction one (the spec names only the former).
    pub message: String,
    /// Where in the document, both from 1.
    pub line: u32,
    pub column: u32,
}

/// Something a stylesheet said that the engine did not apply.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DroppedCss {
    /// The stylesheet's URL; the document's for a `<style>` element.
    pub stylesheet: String,
    pub kind: DroppedCssKind,
    /// A property name, an at-rule's name, or the parser's account of what it could not read.
    pub subject: String,
    /// The line in the stylesheet, from 1. For a `<style>` element, counted from its first line.
    /// For an [`InvalidValue`](DroppedCssKind::InvalidValue) found when an element first used
    /// its rule, the line the rule starts on, not the declaration's.
    pub line: u32,
}

/// Why a [`DroppedCss`] was dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum DroppedCssKind {
    /// A property the engine has no definition for: a misspelling, or one it does not support.
    UnknownProperty,
    /// A value its property's grammar rejects, or that the engine cannot read.
    InvalidValue,
    /// A declaration the parser could not read at all.
    InvalidDeclaration,
    /// A rule whose selector list is invalid or uses a selector the engine does not support.
    InvalidSelector,
    /// An at-rule the engine does not implement, or one in a place it is not allowed.
    UnsupportedAtRule,
}

impl From<CssDroppedKind> for DroppedCssKind {
    fn from(kind: CssDroppedKind) -> Self {
        match kind {
            CssDroppedKind::UnknownProperty => Self::UnknownProperty,
            CssDroppedKind::InvalidValue => Self::InvalidValue,
            CssDroppedKind::InvalidDeclaration => Self::InvalidDeclaration,
            CssDroppedKind::InvalidSelector => Self::InvalidSelector,
            CssDroppedKind::UnsupportedAtRule => Self::UnsupportedAtRule,
        }
    }
}

/// Clamp a position to the `u32` the report carries.
fn narrow(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// Gather `doc`'s report. Only the page's own stylesheets: what the user-agent sheet sets for
/// the engine's own use is not the page's to fix.
pub fn gather<C: RenderConfiguration>(doc: &EngineDocument<C>) -> PageDiagnostics {
    let errors = doc.parse_errors();
    let html_errors = errors
        .iter()
        .take(MAX_HTML_ERRORS)
        .map(|error| HtmlParseError {
            message: error.message.clone(),
            line: narrow(error.location.line),
            column: narrow(error.location.column),
        })
        .collect();

    let mut css_dropped = Vec::new();
    let mut css_dropped_count = 0;
    for sheet in doc
        .stylesheets()
        .iter()
        .filter(|sheet| sheet.origin() == CssOrigin::Author)
    {
        let dropped = sheet.dropped();
        css_dropped_count += dropped.len();
        let room = MAX_CSS_DROPPED.saturating_sub(css_dropped.len());
        css_dropped.extend(dropped.into_iter().take(room).map(|dropped| DroppedCss {
            stylesheet: sheet.url().to_string(),
            kind: dropped.kind.into(),
            subject: dropped.subject,
            line: dropped.line,
        }));
    }

    PageDiagnostics {
        html_errors,
        html_error_count: errors.len(),
        css_dropped,
        css_dropped_count,
    }
}
