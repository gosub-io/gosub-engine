//! The embedder-protocol glue: gdk key translation, the clipboard for Ctrl+C/X/V, the
//! engine-driven mouse cursor, and the desktop colour-scheme probe.

use gosub_engine::events::{EngineEvent, Modifiers, TabCommand};
use gosub_engine::tab::TabHandle;
use gtk4::prelude::*;

/// A gdk key press as the engine's `KeyDown`. GDK key names mostly match DOM
/// `KeyboardEvent.key`; translate the ones that don't. X11 gives Shift+Tab its own
/// `ISO_Left_Tab` keysym.
pub fn key_down_command(key: gtk4::gdk::Key, state: gtk4::gdk::ModifierType) -> Option<TabCommand> {
    let mut modifiers = Modifiers::empty();
    if state.contains(gtk4::gdk::ModifierType::SHIFT_MASK) {
        modifiers |= Modifiers::SHIFT;
    }
    if state.contains(gtk4::gdk::ModifierType::CONTROL_MASK) {
        modifiers |= Modifiers::CONTROL;
    }
    if state.contains(gtk4::gdk::ModifierType::ALT_MASK) {
        modifiers |= Modifiers::ALT;
    }
    if state.contains(gtk4::gdk::ModifierType::SUPER_MASK) {
        modifiers |= Modifiers::META;
    }
    let name = key.name()?;
    let key_name = match name.as_str() {
        "ISO_Left_Tab" => {
            modifiers |= Modifiers::SHIFT;
            "Tab".to_string()
        }
        "Return" | "KP_Enter" => "Enter".to_string(),
        "BackSpace" => "Backspace".to_string(),
        "Left" => "ArrowLeft".to_string(),
        "Right" => "ArrowRight".to_string(),
        "Up" => "ArrowUp".to_string(),
        "Down" => "ArrowDown".to_string(),
        "space" => " ".to_string(),
        _ => match key.to_unicode() {
            Some(c) if !c.is_control() => c.to_string(),
            _ => name.to_string(),
        },
    };
    Some(TabCommand::KeyDown {
        code: key_name.clone(),
        key: key_name,
        modifiers,
    })
}

/// Handle the engine events every GTK example answers the same way; returns whether `evt` was
/// consumed. Clipboard: `ClipboardWrite` lands on the display clipboard, `PasteRequested` is
/// answered with its text as `TextInput` (sent from the GTK main loop - `TabHandle::send` is
/// executor-agnostic). Cursor: `CursorChanged` sets a named cursor on `widget`.
pub fn handle_shell_event(evt: &EngineEvent, widget: &impl IsA<gtk4::Widget>, tab: &TabHandle) -> bool {
    let widget = widget.upcast_ref::<gtk4::Widget>();
    match evt {
        EngineEvent::ClipboardWrite { text, .. } => {
            widget.display().clipboard().set_text(text);
            true
        }
        EngineEvent::PasteRequested { .. } => {
            let tab = tab.clone();
            widget
                .display()
                .clipboard()
                .read_text_async(gtk4::gio::Cancellable::NONE, move |res| {
                    let Ok(Some(text)) = res else {
                        return;
                    };
                    let text = text.to_string();
                    gtk4::glib::spawn_future_local(async move {
                        let _ = tab.send(TabCommand::TextInput { text }).await;
                    });
                });
            true
        }
        EngineEvent::CursorChanged { cursor, .. } => {
            use gosub_engine::events::CursorShape;
            let name = match cursor {
                CursorShape::Default => "default",
                CursorShape::Pointer => "pointer",
                CursorShape::Text => "text",
                CursorShape::Resize => "nwse-resize",
            };
            widget.set_cursor_from_name(Some(name));
            true
        }
        _ => false,
    }
}

fn contains_ci(haystack: &str, needle: &str) -> bool {
    haystack
        .as_bytes()
        .windows(needle.len())
        .any(|w| w.eq_ignore_ascii_case(needle.as_bytes()))
}

fn ends_with_ci(s: &str, suffix: &str) -> bool {
    s.len() >= suffix.len() && s.as_bytes()[s.len() - suffix.len()..].eq_ignore_ascii_case(suffix.as_bytes())
}

/// Whether to render dark: `GOSUB_COLOR_SCHEME` if set, else the desktop's preference.
pub fn desktop_prefers_dark() -> bool {
    if let Ok(v) = std::env::var("GOSUB_COLOR_SCHEME") {
        return v.eq_ignore_ascii_case("dark");
    }
    // GNOME / freedesktop: org.gnome.desktop.interface color-scheme = 'prefer-dark'.
    let gnome_dark = gtk4::gio::SettingsSchemaSource::default()
        .and_then(|src| src.lookup("org.gnome.desktop.interface", true))
        .filter(|schema| schema.has_key("color-scheme"))
        .map(|_| gtk4::gio::Settings::new("org.gnome.desktop.interface").string("color-scheme"))
        .is_some_and(|s| s == "prefer-dark");
    if gnome_dark {
        return true;
    }
    let Some(settings) = gtk4::Settings::default() else {
        return false;
    };
    settings.is_gtk_application_prefer_dark_theme()
        || settings
            .gtk_theme_name()
            .is_some_and(|n| contains_ci(n.as_str(), "dark"))
        || std::env::var("GTK_THEME").is_ok_and(|t| ends_with_ci(&t, ":dark"))
}
