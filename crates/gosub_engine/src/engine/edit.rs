//! Editing form controls: typing into text fields, toggling checkboxes/radios. The state lives
//! on the DOM document (`ControlEditState`, `is_checked`) where selectors and the painter read it.

use crate::html::{EngineDocument, RenderConfiguration};
use cow_utils::CowUtils;
use gosub_interface::document::{ControlEditState, Document as _};
use gosub_shared::node::NodeId;

/// `Some(multiline)` when `id` is an enabled, writable text-entry control.
pub fn text_entry_kind<C: RenderConfiguration>(doc: &EngineDocument<C>, id: NodeId) -> Option<bool> {
    let tag = doc.tag_name(id)?;
    if doc.attribute(id, "disabled").is_some() || doc.attribute(id, "readonly").is_some() {
        return None;
    }
    match tag {
        "textarea" => Some(true),
        "input" => {
            let ty = doc
                .attribute(id, "type")
                .map(|t| t.cow_to_ascii_lowercase().into_owned());
            let typed = matches!(
                ty.as_deref(),
                None | Some("text" | "password" | "search" | "email" | "url" | "tel" | "number")
            );
            typed.then_some(false)
        }
        _ => None,
    }
}

/// Drop the characters a control refuses: `type=number` takes only what can be part of a number
/// (Chrome/Safari behaviour); everything else takes anything.
pub fn filter_insert<C: RenderConfiguration>(doc: &EngineDocument<C>, id: NodeId, text: &str) -> String {
    let numeric = doc.tag_name(id) == Some("input")
        && doc
            .attribute(id, "type")
            .is_some_and(|t| t.eq_ignore_ascii_case("number"));
    if numeric {
        return text
            .chars()
            .filter(|c| c.is_ascii_digit() || matches!(c, '.' | '-' | '+' | 'e' | 'E'))
            .collect();
    }
    // Single-line controls strip line breaks (value sanitization), so a pasted paragraph
    // becomes one line.
    if doc.tag_name(id) != Some("textarea") {
        return text.chars().filter(|c| !matches!(c, '\n' | '\r')).collect();
    }
    text.to_string()
}

/// The markup value: the `value` attribute, or a `<textarea>`'s text content minus the one
/// leading newline HTML allows after the start tag.
pub fn initial_value<C: RenderConfiguration>(doc: &EngineDocument<C>, id: NodeId) -> String {
    if doc.tag_name(id) == Some("textarea") {
        let mut out = String::new();
        for &child in doc.children(id) {
            if let Some(t) = doc.text_value(child) {
                out.push_str(t);
            }
        }
        return out.strip_prefix('\n').unwrap_or(&out).to_string();
    }
    doc.attribute(id, "value").unwrap_or_default().to_string()
}

/// `Some(is_radio)` when `id` is an enabled checkbox or radio button.
pub fn toggle_kind<C: RenderConfiguration>(doc: &EngineDocument<C>, id: NodeId) -> Option<bool> {
    if doc.tag_name(id) != Some("input") || doc.attribute(id, "disabled").is_some() {
        return None;
    }
    match doc
        .attribute(id, "type")
        .map(|t| t.cow_to_ascii_lowercase().into_owned())
        .as_deref()
    {
        Some("checkbox") => Some(false),
        Some("radio") => Some(true),
        _ => None,
    }
}

/// A checkbox flips; a radio becomes checked and the rest of its group (same `name` within the
/// nearest `<form>`, or the document) is unchecked. Returns the `(node, checked)` changes.
pub fn toggle<C: RenderConfiguration>(doc: &EngineDocument<C>, id: NodeId) -> Vec<(NodeId, bool)> {
    match toggle_kind(doc, id) {
        None => Vec::new(),
        Some(false) => vec![(id, !doc.is_checked(id))],
        Some(true) => {
            let mut changes = Vec::new();
            if !doc.is_checked(id) {
                changes.push((id, true));
            }
            let Some(name) = doc.attribute(id, "name").filter(|n| !n.is_empty()) else {
                return changes;
            };
            let scope = crate::engine::form::form_owner(doc, id).unwrap_or_else(|| doc.root());
            let mut stack: Vec<NodeId> = doc.children(scope).iter().rev().copied().collect();
            while let Some(n) = stack.pop() {
                if n != id
                    && toggle_kind(doc, n) == Some(true)
                    && doc.attribute(n, "name") == Some(name)
                    && doc.is_checked(n)
                {
                    changes.push((n, false));
                }
                stack.extend(doc.children(n).iter().rev());
            }
            changes
        }
    }
}

/// Where a caret motion goes. Row-based moves (up/down/page) need the visual rows, so the
/// browsing context resolves those into `Motion::To`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Motion {
    Left,
    Right,
    WordLeft,
    WordRight,
    /// Start of the text (`Home` in a single-line field, `Ctrl+Home` anywhere).
    Start,
    End,
    /// An absolute char index (mouse, row navigation).
    To(usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditAction {
    Insert(String),
    Backspace,
    Delete,
    /// Delete to the previous / next word boundary (Ctrl+Backspace / Ctrl+Delete).
    DeleteWord {
        backwards: bool,
    },
    /// `extend` keeps the anchor (Shift), otherwise the selection collapses.
    Move {
        motion: Motion,
        extend: bool,
    },
    SelectAll,
}

/// Printable keys arrive as their character; Ctrl/Meta chords are shortcuts. Keys that need the
/// visual rows (`ArrowUp`/`ArrowDown`/`PageUp`/`PageDown`, `Home`/`End` in a textarea) are not
/// mapped here.
pub fn action_for_key(key: &str, multiline: bool, ctrl_or_meta: bool, shift: bool) -> Option<EditAction> {
    let mv = |motion| EditAction::Move { motion, extend: shift };
    if ctrl_or_meta {
        return Some(match key {
            "a" | "A" => EditAction::SelectAll,
            "ArrowLeft" => mv(Motion::WordLeft),
            "ArrowRight" => mv(Motion::WordRight),
            "Home" => mv(Motion::Start),
            "End" => mv(Motion::End),
            "Backspace" => EditAction::DeleteWord { backwards: true },
            "Delete" => EditAction::DeleteWord { backwards: false },
            _ => return None,
        });
    }
    Some(match key {
        "Backspace" => EditAction::Backspace,
        "Delete" => EditAction::Delete,
        "ArrowLeft" => mv(Motion::Left),
        "ArrowRight" => mv(Motion::Right),
        "Home" if !multiline => mv(Motion::Start),
        "End" if !multiline => mv(Motion::End),
        "ArrowUp" if !multiline => mv(Motion::Start),
        "ArrowDown" if !multiline => mv(Motion::End),
        "Enter" if multiline => EditAction::Insert("\n".to_string()),
        k if k.chars().count() == 1 && !k.chars().next().is_some_and(char::is_control) => {
            EditAction::Insert(k.to_string())
        }
        _ => return None,
    })
}

fn byte_at(s: &str, ci: usize) -> usize {
    s.char_indices().nth(ci).map_or(s.len(), |(b, _)| b)
}

/// Char index of the previous word start before `caret` (skip spaces, then the word).
pub fn word_left(text: &str, caret: usize) -> usize {
    let chars: Vec<char> = text.chars().collect();
    let mut i = caret.min(chars.len());
    while i > 0 && chars[i - 1].is_whitespace() {
        i -= 1;
    }
    while i > 0 && !chars[i - 1].is_whitespace() {
        i -= 1;
    }
    i
}

/// Char index of the end of the next word after `caret` (skip spaces, then the word), the
/// GTK/Linux convention (Windows would stop at the following word's start).
pub fn word_right(text: &str, caret: usize) -> usize {
    let chars: Vec<char> = text.chars().collect();
    let mut i = caret.min(chars.len());
    while i < chars.len() && chars[i].is_whitespace() {
        i += 1;
    }
    while i < chars.len() && !chars[i].is_whitespace() {
        i += 1;
    }
    i
}

/// The word around char index `at`: a run of non-space chars, or the run of spaces itself.
pub fn word_at(text: &str, at: usize) -> (usize, usize) {
    let chars: Vec<char> = text.chars().collect();
    if chars.is_empty() {
        return (0, 0);
    }
    let at = at.min(chars.len() - 1);
    let space = chars[at].is_whitespace();
    let (mut s, mut e) = (at, at + 1);
    while s > 0 && chars[s - 1].is_whitespace() == space {
        s -= 1;
    }
    while e < chars.len() && chars[e].is_whitespace() == space {
        e += 1;
    }
    (s, e)
}

/// Replace the selection (or `[caret, caret)`) with `text`. Returns whether anything changed.
fn replace_selection(state: &mut ControlEditState, text: &str) -> bool {
    let (start, end) = state.selection().unwrap_or((state.caret, state.caret));
    if start == end && text.is_empty() {
        return false;
    }
    let (bs, be) = (byte_at(&state.value, start), byte_at(&state.value, end));
    state.value.replace_range(bs..be, text);
    state.caret = start + text.chars().count();
    state.anchor = None;
    true
}

/// Returns whether anything changed. Indices are clamped to the value first.
pub fn apply(state: &mut ControlEditState, action: &EditAction) -> bool {
    let len = state.value.chars().count();
    state.caret = state.caret.min(len);
    state.anchor = state.anchor.map(|a| a.min(len)).filter(|a| *a != state.caret);
    match action {
        EditAction::Insert(text) => replace_selection(state, text),
        EditAction::Backspace => {
            if state.selection().is_none() {
                if state.caret == 0 {
                    return false;
                }
                state.anchor = Some(state.caret - 1);
            }
            replace_selection(state, "")
        }
        EditAction::Delete => {
            if state.selection().is_none() {
                if state.caret >= len {
                    return false;
                }
                state.anchor = Some(state.caret + 1);
            }
            replace_selection(state, "")
        }
        EditAction::DeleteWord { backwards } => {
            if state.selection().is_none() {
                let to = if *backwards {
                    word_left(&state.value, state.caret)
                } else {
                    word_right(&state.value, state.caret)
                };
                if to == state.caret {
                    return false;
                }
                state.anchor = Some(to);
            }
            replace_selection(state, "")
        }
        EditAction::Move { motion, extend } => {
            let before = (state.caret, state.selection());
            let sel = state.selection();
            let target = match motion {
                // Without Shift, Left/Right on a selection collapse it to its edge.
                Motion::Left => match sel {
                    Some((s, _)) if !extend => s,
                    _ => state.caret.saturating_sub(1),
                },
                Motion::Right => match sel {
                    Some((_, e)) if !extend => e,
                    _ => (state.caret + 1).min(len),
                },
                Motion::WordLeft => word_left(&state.value, state.caret),
                Motion::WordRight => word_right(&state.value, state.caret),
                Motion::Start => 0,
                Motion::End => len,
                Motion::To(i) => (*i).min(len),
            };
            if *extend {
                state.anchor = state.anchor.or(Some(state.caret));
            } else {
                state.anchor = None;
            }
            state.caret = target;
            state.anchor = state.anchor.filter(|a| *a != state.caret);
            (state.caret, state.selection()) != before
        }
        EditAction::SelectAll => {
            let before = (state.caret, state.anchor);
            state.anchor = (len > 0).then_some(0);
            state.caret = len;
            (state.caret, state.anchor) != before
        }
    }
}

/// `(min, max, step)` of an enabled `<input type=range>`. `step="any"` → a fine step.
pub fn range_params<C: RenderConfiguration>(doc: &EngineDocument<C>, id: NodeId) -> Option<(f64, f64, f64)> {
    if doc.tag_name(id) != Some("input")
        || doc.attribute(id, "disabled").is_some()
        || !doc
            .attribute(id, "type")
            .is_some_and(|t| t.eq_ignore_ascii_case("range"))
    {
        return None;
    }
    let num = |name: &str| doc.attribute(id, name).and_then(|v| v.trim().parse::<f64>().ok());
    let min = num("min").unwrap_or(0.0);
    let max = num("max").unwrap_or(100.0).max(min);
    let step = match doc.attribute(id, "step") {
        Some(s) if s.trim().eq_ignore_ascii_case("any") => (max - min) / 1000.0,
        _ => num("step").filter(|s| *s > 0.0).unwrap_or(1.0),
    };
    Some((min, max, step.max(f64::EPSILON)))
}

/// The slider's current value: what the user dragged to, else the `value` attribute, else the
/// midpoint (HTML's default for range).
pub fn range_value<C: RenderConfiguration>(doc: &EngineDocument<C>, id: NodeId, min: f64, max: f64) -> f64 {
    doc.control_edit_state(id)
        .and_then(|s| s.value.trim().parse::<f64>().ok())
        .or_else(|| doc.attribute(id, "value").and_then(|v| v.trim().parse::<f64>().ok()))
        .unwrap_or((min + max) / 2.0)
        .clamp(min, max)
}

/// Snap `raw` to the step grid anchored at `min`, clamped to the range.
pub fn range_snap(min: f64, max: f64, step: f64, raw: f64) -> f64 {
    let v = min + ((raw - min) / step).round() * step;
    v.clamp(min, max)
}

/// Shortest decimal form: `42`, `7.5`, `0.125`.
pub fn format_number(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        let s = format!("{v:.6}");
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

/// `id` is an enabled `<select>`.
pub fn is_select<C: RenderConfiguration>(doc: &EngineDocument<C>, id: NodeId) -> bool {
    doc.tag_name(id) == Some("select") && doc.attribute(id, "disabled").is_none()
}

/// The enabled options of a `<select>`, in order, through `<optgroup>`s.
pub fn select_options<C: RenderConfiguration>(doc: &EngineDocument<C>, select: NodeId) -> Vec<NodeId> {
    let mut out = Vec::new();
    let mut stack: Vec<NodeId> = doc.children(select).iter().rev().copied().collect();
    while let Some(id) = stack.pop() {
        match doc.tag_name(id) {
            Some("option") if doc.attribute(id, "disabled").is_none() => out.push(id),
            Some("optgroup") => stack.extend(doc.children(id).iter().rev()),
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st(v: &str, caret: usize) -> ControlEditState {
        ControlEditState::new(v.to_string(), caret)
    }

    fn sel(v: &str, anchor: usize, caret: usize) -> ControlEditState {
        ControlEditState {
            anchor: Some(anchor),
            ..st(v, caret)
        }
    }

    fn mv(motion: Motion, extend: bool) -> EditAction {
        EditAction::Move { motion, extend }
    }

    #[test]
    fn insert_and_delete_are_char_based() {
        let mut s = st("héllo", 2);
        assert!(apply(&mut s, &EditAction::Insert("X".into())));
        assert_eq!(s, st("héXllo", 3));
        assert!(apply(&mut s, &EditAction::Backspace));
        assert_eq!(s, st("héllo", 2));
        assert!(apply(&mut s, &EditAction::Backspace));
        assert_eq!(s, st("hllo", 1));
        assert!(apply(&mut s, &EditAction::Delete));
        assert_eq!(s, st("hlo", 1));
    }

    #[test]
    fn caret_movement_clamps() {
        let mut s = st("ab", 0);
        assert!(!apply(&mut s, &mv(Motion::Left, false)));
        assert!(apply(&mut s, &mv(Motion::End, false)));
        assert_eq!(s.caret, 2);
        assert!(!apply(&mut s, &mv(Motion::Right, false)));
        assert!(!apply(&mut s, &EditAction::Delete));
        assert!(apply(&mut s, &mv(Motion::Start, false)));
        assert_eq!(s.caret, 0);
    }

    #[test]
    fn shift_extends_and_plain_moves_collapse() {
        let mut s = st("hello world", 5);
        assert!(apply(&mut s, &mv(Motion::Left, true)));
        assert!(apply(&mut s, &mv(Motion::Left, true)));
        assert_eq!(s.selection(), Some((3, 5)));
        assert_eq!(s.caret, 3);
        // Plain Right collapses to the selection's end, not caret+1.
        assert!(apply(&mut s, &mv(Motion::Right, false)));
        assert_eq!((s.caret, s.selection()), (5, None));
        assert!(apply(&mut s, &mv(Motion::WordRight, true)));
        assert_eq!(s.selection(), Some((5, 11)));
        // Extending back across the anchor flips the selection side.
        assert!(apply(&mut s, &mv(Motion::Start, true)));
        assert_eq!(s.selection(), Some((0, 5)));
    }

    #[test]
    fn typing_replaces_the_selection() {
        let mut s = sel("hello world", 0, 5);
        assert!(apply(&mut s, &EditAction::Insert("bye".into())));
        assert_eq!(s, st("bye world", 3));
        let mut s = sel("hello world", 11, 6);
        assert!(apply(&mut s, &EditAction::Backspace));
        assert_eq!(s, st("hello ", 6));
        let mut s = sel("abc", 1, 2);
        assert!(apply(&mut s, &EditAction::Delete));
        assert_eq!(s, st("ac", 1));
    }

    #[test]
    fn select_all_and_word_deletes() {
        let mut s = st("one two  three", 14);
        assert!(apply(&mut s, &EditAction::SelectAll));
        assert_eq!(s.selection(), Some((0, 14)));
        let mut s = st("one two  three", 14);
        assert!(apply(&mut s, &EditAction::DeleteWord { backwards: true }));
        assert_eq!(s, st("one two  ", 9));
        assert!(apply(&mut s, &EditAction::DeleteWord { backwards: true }));
        assert_eq!(s, st("one ", 4));
        let mut s = st("one two", 0);
        assert!(apply(&mut s, &EditAction::DeleteWord { backwards: false }));
        assert_eq!(s, st(" two", 0));
        assert!(apply(&mut s, &EditAction::DeleteWord { backwards: false }));
        assert_eq!(s, st("", 0));
        assert_eq!(word_at("hello big world", 7), (6, 9));
        assert_eq!(word_at("a  b", 1), (1, 3));
    }

    #[test]
    fn key_mapping() {
        assert_eq!(
            action_for_key("a", false, false, false),
            Some(EditAction::Insert("a".into()))
        );
        assert_eq!(
            action_for_key(" ", false, false, false),
            Some(EditAction::Insert(" ".into()))
        );
        assert_eq!(action_for_key("Enter", false, false, false), None);
        assert_eq!(
            action_for_key("Enter", true, false, false),
            Some(EditAction::Insert("\n".into()))
        );
        assert_eq!(action_for_key("a", false, true, false), Some(EditAction::SelectAll));
        assert_eq!(action_for_key("v", false, true, false), None);
        assert_eq!(action_for_key("Shift", false, false, false), None);
        assert_eq!(
            action_for_key("ArrowLeft", false, true, true),
            Some(mv(Motion::WordLeft, true))
        );
        // Home/End/Up/Down in a textarea are row-based and resolved by the context.
        assert_eq!(action_for_key("Home", true, false, false), None);
        assert_eq!(
            action_for_key("Home", false, false, false),
            Some(mv(Motion::Start, false))
        );
    }
}
