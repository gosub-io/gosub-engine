//! Built-in artwork for checkbox and radio buttons ("Porthole": the Gosub logo's ink outline
//! and cyan→blue face). One SVG per state, scaled into the control's box by the painter.

use crate::common::media::{MediaId, MediaStore, MediaType};

const INK: &str = "#1a0639";
const INK_DISABLED: &str = "#8a8fa0";
const RIM_DISABLED: &str = "#b9bcc9";
const FACE_DISABLED: &str = "#f4f4f7";
const FACE: &str = "url(#g)";
const GRADIENT: &str = r##"<linearGradient id="g" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#60e5ed"/><stop offset="1" stop-color="#2382eb"/></linearGradient>"##;
const GRADIENT_DISABLED: &str = r##"<linearGradient id="g" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#dfe1e8"/><stop offset="1" stop-color="#c3c6d1"/></linearGradient>"##;

/// Media ids of a toggle control's four looks.
#[derive(Debug, Clone, Copy)]
pub struct ToggleIcons {
    pub off: MediaId,
    pub on: MediaId,
    pub off_disabled: MediaId,
    pub on_disabled: MediaId,
}

impl ToggleIcons {
    pub fn pick(&self, checked: bool, disabled: bool) -> MediaId {
        match (checked, disabled) {
            (false, false) => self.off,
            (true, false) => self.on,
            (false, true) => self.off_disabled,
            (true, true) => self.on_disabled,
        }
    }
}

fn checkbox_svg(checked: bool, disabled: bool) -> String {
    let (rim, face, mark, gradient) = if disabled {
        (
            RIM_DISABLED,
            if checked { FACE } else { FACE_DISABLED },
            INK_DISABLED,
            GRADIENT_DISABLED,
        )
    } else {
        (INK, if checked { FACE } else { "#fff" }, INK, GRADIENT)
    };
    let check = if checked {
        format!(
            r#"<path d="M4 7.3 6.3 9.6 10.3 4.8" fill="none" stroke="{mark}" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>"#
        )
    } else {
        String::new()
    };
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 14 14"><defs>{gradient}</defs><rect x="1" y="1" width="12" height="12" rx="3" fill="{face}" stroke="{rim}" stroke-width="2"/>{check}</svg>"#
    )
}

fn radio_svg(checked: bool, disabled: bool) -> String {
    let (rim, face, dot, gradient) = if disabled {
        (
            RIM_DISABLED,
            if checked { FACE } else { FACE_DISABLED },
            INK_DISABLED,
            GRADIENT_DISABLED,
        )
    } else {
        (INK, if checked { FACE } else { "#fff" }, INK, GRADIENT)
    };
    let dot = if checked {
        format!(r#"<circle cx="7" cy="7" r="2.6" fill="{dot}"/>"#)
    } else {
        String::new()
    };
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 14 14"><defs>{gradient}</defs><circle cx="7" cy="7" r="6" fill="{face}" stroke="{rim}" stroke-width="2"/>{dot}</svg>"#
    )
}

/// Dropdown chevron (16px box, 2px stroke); `muted` for a disabled control.
pub fn chevron(store: &MediaStore, muted: bool) -> Option<MediaId> {
    let color = if muted { "#bbbbbb" } else { "#4a4a4a" };
    let svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><path d="M4 6.2 8 10.2 12 6.2" fill="none" stroke="{color}" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/></svg>"##
    );
    store.load_media_from_data(MediaType::Svg, svg.as_bytes()).ok()
}

/// Checkmark for the committed option in a dropdown.
pub fn check(store: &MediaStore, on_blue: bool) -> Option<MediaId> {
    let color = if on_blue { "#ffffff" } else { "#111111" };
    let svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><path d="M3.5 8.5 6.5 11.5 12.5 4.5" fill="none" stroke="{color}" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/></svg>"##
    );
    store.load_media_from_data(MediaType::Svg, svg.as_bytes()).ok()
}

/// A soft drop shadow for a `w`×`h` box with `radius` corners: `0 4px 12px rgba(0,0,0,.12)`,
/// rendered by the SVG rasterizer's Gaussian blur. The image is the box inflated by
/// [`crate::layouter::SELECT_POPUP_SHADOW`].
pub fn drop_shadow(store: &MediaStore, w: f64, h: f64, radius: f64) -> Option<MediaId> {
    let (sx, st, sb) = crate::layouter::SELECT_POPUP_SHADOW;
    let (iw, ih) = (w + sx * 2.0, h + st + sb);
    let svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{iw}" height="{ih}" viewBox="0 0 {iw} {ih}"><defs><filter id="b" x="-20%" y="-20%" width="140%" height="140%"><feGaussianBlur stdDeviation="6"/></filter></defs><rect x="{sx}" y="{}" width="{w}" height="{h}" rx="{radius}" fill="rgba(0,0,0,0.12)" filter="url(#b)"/></svg>"##,
        st + 4.0
    );
    store.load_media_from_data(MediaType::Svg, svg.as_bytes()).ok()
}

/// The textarea resize grip: two short diagonal lines in the corner.
pub fn resize_grip(store: &MediaStore) -> Option<MediaId> {
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 12 12"><path d="M11 5 5 11 M11 9 9 11" fill="none" stroke="#8a8fa0" stroke-width="1.5" stroke-linecap="round"/></svg>"##;
    store.load_media_from_data(MediaType::Svg, svg.as_bytes()).ok()
}

/// Load (or fetch from the store's content cache) the four looks of a checkbox or radio.
pub fn load(store: &MediaStore, radio: bool) -> Option<ToggleIcons> {
    let svg = |checked, disabled| {
        let s = if radio {
            radio_svg(checked, disabled)
        } else {
            checkbox_svg(checked, disabled)
        };
        store.load_media_from_data(MediaType::Svg, s.as_bytes()).ok()
    };
    Some(ToggleIcons {
        off: svg(false, false)?,
        on: svg(true, false)?,
        off_disabled: svg(false, true)?,
        on_disabled: svg(true, true)?,
    })
}
