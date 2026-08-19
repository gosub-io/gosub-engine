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
    if !numeric {
        return text.to_string();
    }
    text.chars()
        .filter(|c| c.is_ascii_digit() || matches!(c, '.' | '-' | '+' | 'e' | 'E'))
        .collect()
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditAction {
    Insert(String),
    Backspace,
    Delete,
    Left,
    Right,
    Home,
    End,
}

/// DOM `KeyboardEvent.key` → edit action. Printable keys arrive as their character; Ctrl/Meta
/// chords are shortcuts, not text.
pub fn action_for_key(key: &str, multiline: bool, ctrl_or_meta: bool) -> Option<EditAction> {
    if ctrl_or_meta {
        return None;
    }
    Some(match key {
        "Backspace" => EditAction::Backspace,
        "Delete" => EditAction::Delete,
        "ArrowLeft" => EditAction::Left,
        "ArrowRight" => EditAction::Right,
        "Home" => EditAction::Home,
        "End" => EditAction::End,
        "Enter" if multiline => EditAction::Insert("\n".to_string()),
        k if k.chars().count() == 1 && !k.chars().next().is_some_and(char::is_control) => {
            EditAction::Insert(k.to_string())
        }
        _ => return None,
    })
}

/// Returns whether anything changed. The caret is a char index.
pub fn apply(state: &mut ControlEditState, action: &EditAction) -> bool {
    let len = state.value.chars().count();
    state.caret = state.caret.min(len);
    let byte_at = |s: &str, ci: usize| s.char_indices().nth(ci).map_or(s.len(), |(b, _)| b);
    match action {
        EditAction::Insert(text) => {
            let at = byte_at(&state.value, state.caret);
            state.value.insert_str(at, text);
            state.caret += text.chars().count();
            true
        }
        EditAction::Backspace => {
            if state.caret == 0 {
                return false;
            }
            let start = byte_at(&state.value, state.caret - 1);
            let end = byte_at(&state.value, state.caret);
            state.value.replace_range(start..end, "");
            state.caret -= 1;
            true
        }
        EditAction::Delete => {
            if state.caret >= len {
                return false;
            }
            let start = byte_at(&state.value, state.caret);
            let end = byte_at(&state.value, state.caret + 1);
            state.value.replace_range(start..end, "");
            true
        }
        EditAction::Left => {
            if state.caret == 0 {
                return false;
            }
            state.caret -= 1;
            true
        }
        EditAction::Right => {
            if state.caret >= len {
                return false;
            }
            state.caret += 1;
            true
        }
        EditAction::Home => {
            let changed = state.caret != 0;
            state.caret = 0;
            changed
        }
        EditAction::End => {
            let changed = state.caret != len;
            state.caret = len;
            changed
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
        ControlEditState {
            value: v.to_string(),
            caret,
        }
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
        assert!(!apply(&mut s, &EditAction::Left) || s.caret == 0);
    }

    #[test]
    fn caret_movement_clamps() {
        let mut s = st("ab", 0);
        assert!(!apply(&mut s, &EditAction::Left));
        assert!(apply(&mut s, &EditAction::End));
        assert_eq!(s.caret, 2);
        assert!(!apply(&mut s, &EditAction::Right));
        assert!(!apply(&mut s, &EditAction::Delete));
        assert!(apply(&mut s, &EditAction::Home));
        assert_eq!(s.caret, 0);
    }

    #[test]
    fn key_mapping() {
        assert_eq!(action_for_key("a", false, false), Some(EditAction::Insert("a".into())));
        assert_eq!(action_for_key(" ", false, false), Some(EditAction::Insert(" ".into())));
        assert_eq!(action_for_key("Enter", false, false), None);
        assert_eq!(
            action_for_key("Enter", true, false),
            Some(EditAction::Insert("\n".into()))
        );
        assert_eq!(action_for_key("a", false, true), None);
        assert_eq!(action_for_key("Shift", false, false), None);
        assert_eq!(action_for_key("Backspace", false, false), Some(EditAction::Backspace));
    }
}
