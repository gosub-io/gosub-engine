//! Colour-scheme preference for engine-drawn UI (native controls, dropdown popup). The CSS side
//! has its own copy of the preference (`gosub_css3::stylesheet::set_prefers_dark`); the engine sets
//! both from one setting.

use crate::painter::commands::color::Color;
use std::sync::atomic::{AtomicBool, Ordering};

static DARK: AtomicBool = AtomicBool::new(false);

pub fn set_dark(dark: bool) {
    DARK.store(dark, Ordering::Relaxed);
}

pub fn is_dark() -> bool {
    DARK.load(Ordering::Relaxed)
}

/// Dropdown tokens from the design guide (light) and their dark counterparts.
pub struct SelectTheme {
    pub popup_bg: Color,
    pub popup_border: Color,
    pub text: Color,
    pub hover_bg: Color,
    pub active_bg: Color,
    pub active_text: Color,
    pub group_text: Color,
    pub disabled_text: Color,
    pub scrollbar_track: Color,
    pub scrollbar_thumb: Color,
    /// Shadow alpha; dark surfaces need a heavier one to read at all.
    pub shadow_alpha: f64,
    /// Chevron / checkmark ink, and the muted variant for a disabled control.
    pub icon: &'static str,
    pub icon_muted: &'static str,
    pub check: &'static str,
}

pub fn select_theme() -> SelectTheme {
    if is_dark() {
        SelectTheme {
            popup_bg: Color::from_rgb8(0x2b, 0x2b, 0x2b),
            popup_border: Color::from_rgb8(0x4a, 0x4a, 0x4a),
            text: Color::from_rgb8(0xee, 0xee, 0xee),
            hover_bg: Color::from_rgb8(0x2f, 0x3c, 0x57),
            active_bg: Color::from_rgb8(0x3a, 0x7a, 0xfe),
            active_text: Color::WHITE,
            group_text: Color::from_rgb8(0x9a, 0x9a, 0x9a),
            disabled_text: Color::from_rgb8(0x6f, 0x6f, 0x6f),
            scrollbar_track: Color::from_rgb8(0x33, 0x33, 0x33),
            scrollbar_thumb: Color::from_rgb8(0x6a, 0x6a, 0x6a),
            shadow_alpha: 0.45,
            icon: "#cfcfcf",
            icon_muted: "#666666",
            check: "#eeeeee",
        }
    } else {
        SelectTheme {
            popup_bg: Color::WHITE,
            popup_border: Color::from_rgb8(0xc7, 0xc7, 0xc7),
            text: Color::from_rgb8(0x11, 0x11, 0x11),
            hover_bg: Color::from_rgb8(0xe6, 0xf0, 0xff),
            active_bg: Color::from_rgb8(0x3a, 0x7a, 0xfe),
            active_text: Color::WHITE,
            group_text: Color::from_rgb8(0x6b, 0x6b, 0x6b),
            disabled_text: Color::from_rgb8(0x88, 0x88, 0x88),
            scrollbar_track: Color::from_rgb8(0xf0, 0xf0, 0xf0),
            scrollbar_thumb: Color::from_rgb8(0xb8, 0xb8, 0xb8),
            shadow_alpha: 0.12,
            icon: "#4a4a4a",
            icon_muted: "#bbbbbb",
            check: "#111111",
        }
    }
}
