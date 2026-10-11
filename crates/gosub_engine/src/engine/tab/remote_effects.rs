//! What the broker does with what an input pass asked for. A renderer is the
//! process a page exploits, so every effect it sends is a request, judged
//! here against what produced it and what a page may do, before the tab
//! worker acts on it. Pure, so the hostile cases are unit tests.

use crate::engine::context::{ClipboardChord, InputProvenance};
use crate::engine::events::{CursorShape, PickerKind};
use crate::fork_server::protocol::{Effect, HitCursor, WireRect};
use http::Method;
use url::Url;

/// What the tab worker does for an effect it believed.
#[derive(Debug, PartialEq)]
pub(crate) enum Action {
    Focus {
        focused: bool,
        editable: bool,
    },
    Cursor(CursorShape),
    Navigate {
        url: Url,
        method: Method,
        /// The form-encoded body of a POST.
        body: Option<String>,
        /// The referrer policy the link or form asked for; `None` leaves the document's.
        referrer_policy: Option<gosub_sonar::ReferrerPolicy>,
    },
    Picker {
        kind: PickerKind,
        bounds: WireRect,
        value: String,
        min: Option<String>,
        max: Option<String>,
        step: Option<String>,
    },
    ClipboardWrite(String),
    PasteRequested,
    Capture(bool),
}

/// Judge one effect. `current` is the page the input went to, which a
/// navigation resolves against and whose scheme decides what it may reach;
/// `viewport` bounds any rectangle. `Err` names what was refused, for the log.
pub(crate) fn action_for(
    effect: Effect,
    provenance: InputProvenance,
    current: Option<&Url>,
    viewport: (f64, f64),
) -> Result<Action, &'static str> {
    match effect {
        Effect::Focus { focused, editable, .. } => Ok(Action::Focus { focused, editable }),
        Effect::Cursor { cursor } => {
            if !provenance.pointer {
                return Err("a cursor from a pass no pointer produced");
            }
            Ok(Action::Cursor(match cursor {
                HitCursor::Default => CursorShape::Default,
                HitCursor::Pointer => CursorShape::Pointer,
                HitCursor::Text => CursorShape::Text,
                HitCursor::Resize => CursorShape::Resize,
            }))
        }
        Effect::Navigate {
            url,
            post,
            body,
            referrer_policy,
        } => {
            // The same rule a hit region's link gets: http and https, and file
            // only from a file page.
            let current = current.ok_or("a navigation with no page to navigate from")?;
            let url = current
                .join(&url)
                .map_err(|_| "a navigation to a URL that does not parse")?;
            if !crate::engine::tab::page_may_navigate(current, &url) {
                return Err("a navigation to a scheme a page may not reach");
            }
            Ok(Action::Navigate {
                url,
                method: if post { Method::POST } else { Method::GET },
                body: if post { body } else { None },
                // The page's own choice of how much of its URL a navigation it
                // chose reveals: what holding the page means, like the target.
                referrer_policy: referrer_policy.map(Into::into),
            })
        }
        Effect::Picker {
            kind,
            bounds,
            value,
            min,
            max,
            step,
        } => Ok(Action::Picker {
            kind,
            bounds: clamp(bounds, viewport),
            value,
            min,
            max,
            step,
        }),
        Effect::ClipboardWrite { text } => {
            if !matches!(provenance.chord, ClipboardChord::Copy | ClipboardChord::Cut) {
                return Err("a clipboard write from a pass no copy or cut chord produced");
            }
            Ok(Action::ClipboardWrite(text))
        }
        Effect::PasteRequested => {
            if provenance.chord != ClipboardChord::Paste {
                return Err("a paste request from a pass no paste chord produced");
            }
            Ok(Action::PasteRequested)
        }
        Effect::Capture { pointer } => Ok(Action::Capture(pointer)),
    }
}

/// A rectangle the embedder places a window over: inside the viewport,
/// whatever the renderer said.
fn clamp(r: WireRect, (vw, vh): (f64, f64)) -> WireRect {
    let x = r.x.clamp(0.0, vw.max(0.0));
    let y = r.y.clamp(0.0, vh.max(0.0));
    WireRect {
        x,
        y,
        width: r.width.clamp(0.0, (vw - x).max(0.0)),
        height: r.height.clamp(0.0, (vh - y).max(0.0)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fork_server::protocol::InputEvent;

    fn page() -> Url {
        Url::parse("https://page.test/form").unwrap()
    }
    fn pointer() -> InputProvenance {
        InputProvenance::of(&InputEvent::PointerDown {
            x: 1.0,
            y: 1.0,
            button: crate::engine::events::MouseButton::Left,
        })
    }
    fn key(key: &str, modifiers: u8) -> InputProvenance {
        InputProvenance::of(&InputEvent::KeyDown {
            key: key.into(),
            modifiers,
        })
    }
    const CONTROL: u8 = 0b0010;

    #[test]
    fn a_navigation_is_held_to_what_a_link_may_reach() {
        let nav = |url: &str, post: bool| Effect::Navigate {
            url: url.into(),
            post,
            body: Some("a=1".into()),
            referrer_policy: Some(crate::fork_server::protocol::WireReferrerPolicy::NoReferrer),
        };
        assert!(matches!(
            action_for(nav("/submit?x=1", false), key("Enter", 0), Some(&page()), (800.0, 600.0)),
            Ok(Action::Navigate {
                url,
                method: Method::GET,
                body: None,
                referrer_policy: Some(gosub_sonar::ReferrerPolicy::NoReferrer),
            }) if url.as_str() == "https://page.test/submit?x=1"
        ));
        assert!(matches!(
            action_for(nav("/submit", true), key("Enter", 0), Some(&page()), (800.0, 600.0)),
            Ok(Action::Navigate { method: Method::POST, body: Some(b), .. }) if b == "a=1"
        ));
        assert!(action_for(
            nav("javascript:alert(1)", false),
            key("Enter", 0),
            Some(&page()),
            (800.0, 600.0)
        )
        .is_err());
        assert!(action_for(
            nav("file:///etc/passwd", false),
            key("Enter", 0),
            Some(&page()),
            (800.0, 600.0)
        )
        .is_err());
        assert!(action_for(nav("/x", false), key("Enter", 0), None, (800.0, 600.0)).is_err());
        let local = Url::parse("file:///home/u/page.html").unwrap();
        assert!(action_for(nav("other.html", false), key("Enter", 0), Some(&local), (800.0, 600.0)).is_ok());
    }

    #[test]
    fn a_cursor_is_believed_from_the_pointer_only() {
        let cursor = Effect::Cursor {
            cursor: HitCursor::Text,
        };
        assert_eq!(
            action_for(cursor.clone(), pointer(), Some(&page()), (800.0, 600.0)),
            Ok(Action::Cursor(CursorShape::Text))
        );
        assert!(action_for(cursor, key("a", 0), Some(&page()), (800.0, 600.0)).is_err());
    }

    #[test]
    fn clipboard_effects_need_the_chord_that_asks_for_them() {
        let write = Effect::ClipboardWrite { text: "secret".into() };
        assert!(action_for(write.clone(), key("c", CONTROL), Some(&page()), (800.0, 600.0)).is_ok());
        assert!(action_for(write.clone(), key("x", CONTROL), Some(&page()), (800.0, 600.0)).is_ok());
        assert!(action_for(write.clone(), key("a", CONTROL), Some(&page()), (800.0, 600.0)).is_err());
        assert!(action_for(write.clone(), key("c", 0), Some(&page()), (800.0, 600.0)).is_err());
        assert!(action_for(write, pointer(), Some(&page()), (800.0, 600.0)).is_err());
        assert!(action_for(Effect::PasteRequested, key("v", CONTROL), Some(&page()), (800.0, 600.0)).is_ok());
        assert!(action_for(Effect::PasteRequested, key("c", CONTROL), Some(&page()), (800.0, 600.0)).is_err());
    }

    #[test]
    fn a_picker_lands_inside_the_viewport() {
        let picker = Effect::Picker {
            kind: PickerKind::Date,
            bounds: WireRect {
                x: 790.0,
                y: -20.0,
                width: 100.0,
                height: 50.0,
            },
            value: "2026-10-03".into(),
            min: None,
            max: None,
            step: None,
        };
        let Ok(Action::Picker { bounds, .. }) = action_for(picker, pointer(), Some(&page()), (800.0, 600.0)) else {
            panic!("a picker is always believed");
        };
        assert_eq!(
            bounds,
            WireRect {
                x: 790.0,
                y: 0.0,
                width: 10.0,
                height: 50.0
            }
        );
    }
}
