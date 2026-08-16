//! Text editing in form controls: which controls take typed input, and how a key press changes
//! a control's value and caret. The state itself lives on the DOM document (see
//! `ControlEditState`) so layout/paint read the same value the user sees.

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

/// The value a control shows before the user touches it: the `value` attribute for `<input>`,
/// the text content for `<textarea>` (minus the single leading newline HTML allows after the
/// start tag).
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

/// An editing action derived from a key press.
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

/// Map a DOM `KeyboardEvent.key` value to an edit action. Printable keys arrive as their
/// character (a single grapheme); `ctrl`/`meta` chords are shortcuts, not text.
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

/// Apply `action` to `state`; returns whether anything changed. The caret is a char index.
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
