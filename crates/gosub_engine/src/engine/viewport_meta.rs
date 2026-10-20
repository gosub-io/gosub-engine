//! `<meta name="viewport">`: what width a page is laid out at on a small screen, and at what
//! scale it is shown.
//!
//! A phone that laid every page out at its own width would squeeze pages written for a desktop
//! window into a column a third as wide. Mobile browsers lay a page out at a desktop-like width
//! instead and scale the result down to fit, unless the page says, with this tag, that it was
//! written for the device's width.
//!
//! css-viewport-1 defines how the `content` attribute is parsed, and the property names; how the
//! values combine into a width and a scale it leaves unwritten ("Specify me"). The rules here are
//! the ones css-device-adapt-1 (2016 Working Draft, §6.2 and §9) wrote down: each property
//! becomes an `@viewport` descriptor, the UA's own `@viewport { width: extend-to-zoom 980px;
//! min-zoom: 0.25; max-zoom: 5 }` sits under them, and the constraining procedure resolves the
//! result against the screen.
//!
//! Only a mobile embedder asks for any of this (`useragent.viewport.mobile`). Everywhere else
//! the layout viewport is the window, as it has always been.

/// The width the UA lays a page out at when the page does not say: css-device-adapt-1 §9's
/// example UA sheet, and what every mobile browser uses.
pub const FALLBACK_LAYOUT_WIDTH: f64 = 980.0;

/// The smallest and largest scale the UA allows, from the same UA sheet.
const UA_MIN_ZOOM: f64 = 0.25;
const UA_MAX_ZOOM: f64 = 5.0;

/// One value of a `width` or `height` property.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Extent {
    /// CSS px, clamped to `[1, 10000]`.
    Px(f64),
    /// `device-width`: 100vw of the screen.
    DeviceWidth,
    /// `device-height`: 100vh of the screen.
    DeviceHeight,
}

/// The properties of a viewport `<meta>` that decide the layout viewport and its scale, as
/// css-device-adapt-1 §9.4 translates them. `None` is a property the tag did not set, or set to
/// a value the translation drops (a negative number).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ViewportMeta {
    pub width: Option<Extent>,
    pub height: Option<Extent>,
    pub initial_scale: Option<f64>,
    pub minimum_scale: Option<f64>,
    pub maximum_scale: Option<f64>,
    pub user_scalable: Option<bool>,
}

/// A value as Set-Property classifies it: a number when a prefix of it reads as one, else one of
/// the four keywords, else unknown.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Value {
    Number(f64),
    Yes,
    No,
    DeviceWidth,
    DeviceHeight,
    Unknown,
}

impl ViewportMeta {
    /// Parse a `content` attribute with css-viewport-1's Parse-Content: pairs separated by `,`
    /// (or `;`, which some UAs accept), names and values split by `=`, whitespace around either
    /// ignored. A later value for the same property wins.
    pub fn parse(content: &str) -> Self {
        let mut meta = Self::default();
        let s = content.as_bytes();
        let is_space = |c: u8| matches!(c, b'\t' | b'\n' | b'\r' | b' ');
        let is_separator = |c: u8| matches!(c, b',' | b';');
        let mut i = 0;
        while i < s.len() {
            while i < s.len() && (is_space(s[i]) || is_separator(s[i]) || s[i] == b'=') {
                i += 1;
            }
            if i >= s.len() {
                break;
            }
            // Parse-Property: a name, anything up to `=`, then the value.
            let start = i;
            while i < s.len() && !(is_space(s[i]) || is_separator(s[i]) || s[i] == b'=') {
                i += 1;
            }
            if i >= s.len() || is_separator(s[i]) {
                continue;
            }
            let name = &content[start..i];
            while i < s.len() && !(is_separator(s[i]) || s[i] == b'=') {
                i += 1;
            }
            if i >= s.len() || is_separator(s[i]) {
                continue;
            }
            while i < s.len() && (is_space(s[i]) || s[i] == b'=') {
                i += 1;
            }
            if i >= s.len() || is_separator(s[i]) {
                continue;
            }
            let start = i;
            while i < s.len() && !(is_space(s[i]) || is_separator(s[i]) || s[i] == b'=') {
                i += 1;
            }
            meta.set(name, classify(&content[start..i]));
        }
        meta
    }

    /// Set-Property, with css-device-adapt-1 §9.4's translation of each value.
    fn set(&mut self, name: &str, value: Value) {
        let extent = |value: Value| match value {
            Value::Number(n) if n < 0.0 => None,
            Value::Number(n) => Some(Extent::Px(n.clamp(1.0, 10_000.0))),
            Value::DeviceWidth => Some(Extent::DeviceWidth),
            Value::DeviceHeight => Some(Extent::DeviceHeight),
            // Other keywords and unknown values are 1px.
            Value::Yes | Value::No | Value::Unknown => Some(Extent::Px(1.0)),
        };
        let scale = |value: Value| match value {
            Value::Number(n) if n < 0.0 => None,
            Value::Number(n) => Some(n.clamp(0.1, 10.0)),
            Value::Yes => Some(1.0),
            Value::DeviceWidth | Value::DeviceHeight => Some(10.0),
            Value::No | Value::Unknown => Some(0.1),
        };
        if name.eq_ignore_ascii_case("width") {
            if let Some(width) = extent(value) {
                self.width = Some(width);
            }
        } else if name.eq_ignore_ascii_case("height") {
            if let Some(height) = extent(value) {
                self.height = Some(height);
            }
        } else if name.eq_ignore_ascii_case("initial-scale") {
            if let Some(zoom) = scale(value) {
                self.initial_scale = Some(zoom);
            }
        } else if name.eq_ignore_ascii_case("minimum-scale") {
            if let Some(zoom) = scale(value) {
                self.minimum_scale = Some(zoom);
            }
        } else if name.eq_ignore_ascii_case("maximum-scale") {
            if let Some(zoom) = scale(value) {
                self.maximum_scale = Some(zoom);
            }
        } else if name.eq_ignore_ascii_case("user-scalable") {
            self.user_scalable = Some(match value {
                Value::Yes | Value::DeviceWidth | Value::DeviceHeight => true,
                Value::Number(n) => n.abs() >= 1.0,
                Value::No | Value::Unknown => false,
            });
        }
        // `interactive-widget` concerns the on-screen keyboard, which does not resize anything
        // here yet; every other name is ignored, as Set-Property says.
    }
}

/// Set-Property's classification: a number when a prefix of the value converts with `strtod`,
/// otherwise a keyword matched case-insensitively.
fn classify(value: &str) -> Value {
    if let Some(n) = number_prefix(value) {
        return Value::Number(n);
    }
    if value.eq_ignore_ascii_case("yes") {
        Value::Yes
    } else if value.eq_ignore_ascii_case("no") {
        Value::No
    } else if value.eq_ignore_ascii_case("device-width") {
        Value::DeviceWidth
    } else if value.eq_ignore_ascii_case("device-height") {
        Value::DeviceHeight
    } else {
        Value::Unknown
    }
}

/// The longest prefix of `value` that `strtod` reads as a decimal number: an optional sign,
/// digits with at most one `.` (at least one digit in all), and an optional exponent. `300px` is
/// 300. `strtod`'s hexadecimal and `inf`/`nan` forms are left out: no page writes a width as
/// either, and an infinite one would only be clamped.
fn number_prefix(value: &str) -> Option<f64> {
    let s = value.as_bytes();
    let mut i = 0;
    if i < s.len() && matches!(s[i], b'+' | b'-') {
        i += 1;
    }
    let mut digits = 0;
    while i < s.len() && s[i].is_ascii_digit() {
        i += 1;
        digits += 1;
    }
    if i < s.len() && s[i] == b'.' {
        i += 1;
        while i < s.len() && s[i].is_ascii_digit() {
            i += 1;
            digits += 1;
        }
    }
    if digits == 0 {
        return None;
    }
    // An exponent only counts when digits follow it: `2e` is 2.
    if i < s.len() && matches!(s[i], b'e' | b'E') {
        let mut j = i + 1;
        if j < s.len() && matches!(s[j], b'+' | b'-') {
            j += 1;
        }
        if j < s.len() && s[j].is_ascii_digit() {
            while j < s.len() && s[j].is_ascii_digit() {
                j += 1;
            }
            i = j;
        }
    }
    value[..i].parse().ok()
}

/// The layout viewport a page gets on a given screen, and the scale it is shown at.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PageScale {
    /// The layout viewport, in CSS px: the initial containing block, and what `width` and
    /// `height` media features report.
    pub layout_width: u32,
    pub layout_height: u32,
    /// Screen CSS px per layout CSS px. 1 when the page is laid out at the screen's width.
    pub scale: f64,
    /// The range a user may zoom within, and whether they may at all.
    pub min_scale: f64,
    pub max_scale: f64,
    pub user_scalable: bool,
}

impl PageScale {
    /// No viewport handling: the page is laid out at the screen's size, unscaled. What a desktop
    /// window gets.
    pub fn identity(screen_width: u32, screen_height: u32) -> Self {
        Self {
            layout_width: screen_width,
            layout_height: screen_height,
            scale: 1.0,
            min_scale: UA_MIN_ZOOM,
            max_scale: UA_MAX_ZOOM,
            user_scalable: true,
        }
    }

    /// Resolve the layout viewport for a screen of `screen_width` x `screen_height` CSS px and
    /// the page's viewport `<meta>`, if it has one: css-device-adapt-1's constraining procedure
    /// over the UA sheet and the descriptors §9.4 makes of the tag.
    ///
    /// The page is always shown at the scale that fits its layout width to the screen. Where the
    /// tag asks for another initial scale (`width=980, initial-scale=1` on a phone, or a scale
    /// clamped by `minimum-scale`), the browser would show part of the layout viewport and let
    /// the user pan; that needs a visual viewport separate from the layout one, which the engine
    /// does not have yet, so the scale falls back to fitting the width.
    pub fn resolve(screen_width: u32, screen_height: u32, meta: Option<&ViewportMeta>) -> Self {
        if screen_width == 0 || screen_height == 0 {
            return Self::identity(screen_width, screen_height);
        }
        let initial_width = f64::from(screen_width);
        let initial_height = f64::from(screen_height);

        // The cascade: the UA sheet, then the tag's descriptors over it.
        let mut min_width = Descriptor::ExtendToZoom;
        let mut max_width = Descriptor::Px(FALLBACK_LAYOUT_WIDTH);
        let mut min_height = Descriptor::Auto;
        let mut max_height = Descriptor::Auto;
        let mut zoom = None;
        let mut min_zoom = Some(UA_MIN_ZOOM);
        let mut max_zoom = Some(UA_MAX_ZOOM);
        let mut user_scalable = true;
        if let Some(meta) = meta {
            let length = |extent: Extent| match extent {
                Extent::Px(px) => Descriptor::Px(px),
                Extent::DeviceWidth => Descriptor::Px(initial_width),
                Extent::DeviceHeight => Descriptor::Px(initial_height),
            };
            if let Some(width) = meta.width {
                min_width = Descriptor::ExtendToZoom;
                max_width = length(width);
            }
            if let Some(height) = meta.height {
                min_height = Descriptor::ExtendToZoom;
                max_height = length(height);
            }
            // `initial-scale` without a width: the width extends to what that scale shows,
            // unless a height was given, which then decides the width.
            if meta.width.is_none() && meta.initial_scale.is_some() {
                let width = if meta.height.is_some() {
                    Descriptor::Auto
                } else {
                    Descriptor::ExtendToZoom
                };
                min_width = width;
                max_width = width;
            }
            if let Some(scale) = meta.initial_scale {
                zoom = Some(scale);
            }
            if let Some(scale) = meta.minimum_scale {
                min_zoom = Some(scale);
            }
            if let Some(scale) = meta.maximum_scale {
                max_zoom = Some(scale);
            }
            if let Some(scalable) = meta.user_scalable {
                user_scalable = scalable;
            }
        }

        // Steps 1 and 2: the zoom range, and the zoom inside it.
        if let (Some(min), Some(max)) = (min_zoom, max_zoom) {
            max_zoom = Some(max.max(min));
        }
        if let Some(z) = zoom {
            let z = max_zoom.map_or(z, |max| z.min(max));
            zoom = Some(min_zoom.map_or(z, |min| z.max(min)));
        }

        // Step 3: extend-to-zoom, max descriptors before min ones.
        let extend_zoom = match (zoom, max_zoom) {
            (Some(z), Some(max)) => Some(z.min(max)),
            (z, _) => z,
        };
        let (extend_width, extend_height) = match extend_zoom {
            Some(z) => (Descriptor::Px(initial_width / z), Descriptor::Px(initial_height / z)),
            None => (Descriptor::Auto, Descriptor::Auto),
        };
        let resolve_max = |max: Descriptor, extend: Descriptor| match max {
            Descriptor::ExtendToZoom => extend,
            other => other,
        };
        let resolve_min = |min: Descriptor, max: Descriptor, extend: Descriptor| match (min, extend) {
            (Descriptor::ExtendToZoom, Descriptor::Px(e)) => match max {
                Descriptor::Px(m) => Descriptor::Px(e.max(m)),
                _ => Descriptor::Px(e),
            },
            (Descriptor::ExtendToZoom, _) => max,
            (other, _) => other,
        };
        let max_width = resolve_max(max_width, extend_width);
        let max_height = resolve_max(max_height, extend_height);
        let min_width = resolve_min(min_width, max_width, extend_width);
        let min_height = resolve_min(min_height, max_height, extend_height);

        // Steps 4 to 6: width and height from their ranges, the missing one from the screen's
        // aspect ratio.
        let clamp = |min: Descriptor, max: Descriptor, initial: f64| -> Option<f64> {
            if matches!((min, max), (Descriptor::Auto, Descriptor::Auto)) {
                return None;
            }
            let upper = max.px().map_or(initial, |m| initial.min(m));
            Some(min.px().map_or(upper, |m| m.max(upper)))
        };
        let width = match clamp(min_width, max_width, initial_width) {
            Some(width) => width,
            None => clamp(min_height, max_height, initial_height)
                .map_or(initial_width, |height| height * initial_width / initial_height),
        };

        let (min_scale, max_scale) = (min_zoom.unwrap_or(UA_MIN_ZOOM), max_zoom.unwrap_or(UA_MAX_ZOOM));
        // The scale that fits the width, no larger than `max-zoom`. A layout narrower than that
        // shows (`width=1`, or `width=foo`, which §9.4 makes 1px) extends to what the screen holds
        // at the largest scale allowed, as the procedure resolves `zoom: auto`, so that the
        // layout viewport still fills the screen.
        let width = width.max(initial_width / max_scale);
        let scale = initial_width / width;
        let layout_width = width.round().max(1.0) as u32;
        // The layout viewport keeps the screen's shape at the fitted scale. A `height` that
        // gives it another shape would leave part of the screen showing neither, which takes a
        // visual viewport to handle; until then a height only counts where it decides the width.
        let layout_height = (initial_height / scale).round().max(1.0) as u32;
        Self {
            layout_width,
            layout_height,
            scale,
            min_scale,
            max_scale,
            user_scalable,
        }
    }
}

/// A width or height descriptor while the constraining procedure resolves it.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Descriptor {
    Auto,
    ExtendToZoom,
    Px(f64),
}

impl Descriptor {
    fn px(self) -> Option<f64> {
        match self {
            Descriptor::Px(px) => Some(px),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(content: &str) -> ViewportMeta {
        ViewportMeta::parse(content)
    }

    fn layout(screen: (u32, u32), content: Option<&str>) -> (u32, u32, f64) {
        let meta = content.map(meta);
        let page = PageScale::resolve(screen.0, screen.1, meta.as_ref());
        (
            page.layout_width,
            page.layout_height,
            (page.scale * 1000.0).round() / 1000.0,
        )
    }

    #[test]
    fn the_content_attribute_is_parsed_as_css_viewport_says() {
        let m = meta("width=device-width, initial-scale=1");
        assert_eq!(m.width, Some(Extent::DeviceWidth));
        assert_eq!(m.initial_scale, Some(1.0));

        // Semicolons separate too, names are case-insensitive, whitespace around `=` is fine.
        let m = meta(" WIDTH = 500 ;Initial-Scale=0.5");
        assert_eq!(m.width, Some(Extent::Px(500.0)));
        assert_eq!(m.initial_scale, Some(0.5));

        // A number prefix is the number; the rest of the value is ignored.
        assert_eq!(meta("width=300px").width, Some(Extent::Px(300.0)));
        assert_eq!(meta("initial-scale=1.0abc").initial_scale, Some(1.0));
        assert_eq!(meta("initial-scale=2e").initial_scale, Some(2.0));

        // The later value wins.
        assert_eq!(meta("width=400, width=600").width, Some(Extent::Px(600.0)));

        // A name with no value sets nothing.
        assert_eq!(meta("width, initial-scale=1").width, None);
        assert_eq!(meta("width=").width, None);
    }

    #[test]
    fn values_translate_as_css_device_adapt_says() {
        // Lengths clamp to [1, 10000]; a negative one is dropped; unknown words are 1px.
        assert_eq!(meta("width=20000").width, Some(Extent::Px(10_000.0)));
        assert_eq!(meta("width=-5").width, None);
        assert_eq!(meta("width=banana").width, Some(Extent::Px(1.0)));
        // Scales clamp to [0.1, 10]; `yes` is 1, `no` 0.1, the device keywords 10.
        assert_eq!(meta("initial-scale=50").initial_scale, Some(10.0));
        assert_eq!(meta("initial-scale=yes").initial_scale, Some(1.0));
        assert_eq!(meta("initial-scale=no").initial_scale, Some(0.1));
        assert_eq!(meta("maximum-scale=device-width").maximum_scale, Some(10.0));
        assert_eq!(meta("minimum-scale=-1").minimum_scale, None);
        // user-scalable: yes, the device keywords and |n| >= 1 allow zooming.
        assert_eq!(meta("user-scalable=no").user_scalable, Some(false));
        assert_eq!(meta("user-scalable=0").user_scalable, Some(false));
        assert_eq!(meta("user-scalable=-2").user_scalable, Some(true));
        assert_eq!(meta("user-scalable=1").user_scalable, Some(true));
    }

    #[test]
    fn a_page_without_the_tag_is_laid_out_at_980px_and_scaled_to_fit() {
        assert_eq!(layout((360, 780), None), (980, 2123, 0.367));
        // The UA sheet's width holds on a wider screen too; it is shown larger.
        assert_eq!(layout((1280, 800), None), (980, 613, 1.306));
        // A tag that says nothing about width or scale changes nothing.
        assert_eq!(layout((360, 780), Some("user-scalable=no")), (980, 2123, 0.367));
    }

    #[test]
    fn device_width_lays_the_page_out_at_the_screen() {
        assert_eq!(layout((360, 780), Some("width=device-width")), (360, 780, 1.0));
        assert_eq!(
            layout((360, 780), Some("width=device-width, initial-scale=1")),
            (360, 780, 1.0)
        );
        // `initial-scale=1` alone extends the width to what that scale shows.
        assert_eq!(layout((360, 780), Some("initial-scale=1")), (360, 780, 1.0));
        assert_eq!(layout((360, 780), Some("initial-scale=2")), (180, 390, 2.0));
    }

    #[test]
    fn a_fixed_width_is_scaled_to_fit() {
        assert_eq!(layout((360, 780), Some("width=500")), (500, 1083, 0.72));
        // A width narrower than what `initial-scale` shows is extended to it.
        assert_eq!(layout((360, 780), Some("width=320, initial-scale=1")), (360, 780, 1.0));
        // Wider than the screen at scale 1: laid out at its width, and (until there is a visual
        // viewport to pan) shown fitted rather than at scale 1.
        assert_eq!(
            layout((360, 780), Some("width=980, initial-scale=1")),
            (980, 2123, 0.367)
        );
    }

    /// A layout narrower than the screen shows at `max-zoom` extends to fill it at that scale.
    #[test]
    fn the_scale_stops_at_max_zoom() {
        // An unknown width is 1px; the UA's max-zoom of 5 shows 72px of a 360px screen.
        assert_eq!(layout((360, 780), Some("width=foo")), (72, 156, 5.0));
        assert_eq!(layout((360, 780), Some("width=1")), (72, 156, 5.0));
        // The page's own maximum counts too.
        assert_eq!(layout((360, 780), Some("width=100, maximum-scale=2")), (180, 390, 2.0));
        // Below the cap a narrow width keeps its fitted scale.
        assert_eq!(layout((360, 780), Some("width=120")), (120, 260, 3.0));
    }

    #[test]
    fn the_zoom_range_is_reported() {
        let page = PageScale::resolve(
            360,
            780,
            Some(&meta(
                "width=device-width, minimum-scale=1, maximum-scale=1, user-scalable=no",
            )),
        );
        assert_eq!((page.min_scale, page.max_scale, page.user_scalable), (1.0, 1.0, false));
        // A maximum below the minimum is raised to it.
        let page = PageScale::resolve(360, 780, Some(&meta("minimum-scale=2, maximum-scale=1")));
        assert_eq!((page.min_scale, page.max_scale), (2.0, 2.0));
    }
}
