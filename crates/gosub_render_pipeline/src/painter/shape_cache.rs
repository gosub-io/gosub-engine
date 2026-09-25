//! Shaped text kept between paint passes.
//!
//! Shaping is what a paint pass spends its time on: a text run costs hundreds of microseconds
//! to shape, against a few hundred nanoseconds for everything else the painter reads per
//! element. A pass shapes each element once (the painter's own per-pass memo), but a scroll
//! repaints the runs that straddle the window edge pass after pass, and on a long article that
//! is half of all the shaping done.

use std::collections::HashMap;
use std::sync::Arc;

use gosub_interface::font::FontStyle;
use gosub_interface::font_system::{ShapedText, TextAlign, TextStyle};
use parking_lot::Mutex;

/// How many shaped runs one cache holds. A grid is repainted window by window, so the runs that
/// repeat do so within a few passes of each other and a bounded cache still catches them; the
/// cap is what keeps a 29 000 px article from holding every glyph of every line it ever painted.
const MAX_ENTRIES: usize = 8192;

/// Everything [`gosub_interface::font_system::FontSystem::shape`] reads, in a hashable form.
/// Floats go in as their bit patterns: they are copied through from the computed style, never
/// arithmetic results, so equal styles give equal bits.
#[derive(PartialEq, Eq, Hash)]
struct ShapeKey {
    text: String,
    family: String,
    size: u32,
    weight: u16,
    style: u8,
    stretch: u32,
    line_height: Option<u32>,
    letter_spacing: u32,
    max_width: Option<u32>,
    align: u8,
    display_scale: u32,
}

impl ShapeKey {
    fn new(text: &str, style: &TextStyle) -> Self {
        ShapeKey {
            text: text.to_string(),
            family: style.family.clone(),
            size: style.size.to_bits(),
            weight: style.weight.0,
            style: match style.style {
                FontStyle::Normal => 0,
                FontStyle::Italic => 1,
                FontStyle::Oblique => 2,
            },
            stretch: style.stretch.0.to_bits(),
            line_height: style.line_height.map(f32::to_bits),
            letter_spacing: style.letter_spacing.to_bits(),
            max_width: style.max_width.map(f32::to_bits),
            align: match style.align {
                TextAlign::Start => 0,
                TextAlign::Center => 1,
                TextAlign::End => 2,
                TextAlign::Justify => 3,
            },
            display_scale: style.display_scale.to_bits(),
        }
    }
}

/// Text already shaped for the tile grid that owns this cache.
///
/// Keyed by the string and the style it is shaped in, not by element id: the same run of text
/// belongs to several elements as often as it belongs to several tiles, and an element's text
/// and style change under it (a typed character, a hover rule) without its id changing. A key
/// is only worth as much as the font set it was shaped against, which is why the cache belongs
/// to the tile grid - the engine registers every web font before the first layout, so no grid
/// spans a change to the font set.
#[derive(Default)]
pub struct ShapeCache {
    entries: Mutex<HashMap<ShapeKey, Arc<ShapedText>>>,
}

impl std::fmt::Debug for ShapeCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ShapeCache")
            .field("entries", &self.entries.lock().len())
            .finish()
    }
}

impl ShapeCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// The shaped form of `text` in `style`, shaping it with `shape` on a miss.
    ///
    /// `shape` runs without the cache locked: it takes the font system's own lock, and holding
    /// two locks to shape one run is how a paint pass and a layout would deadlock.
    pub fn get_or_shape(&self, text: &str, style: &TextStyle, shape: impl FnOnce() -> ShapedText) -> Arc<ShapedText> {
        let key = ShapeKey::new(text, style);
        if let Some(hit) = self.entries.lock().get(&key) {
            return Arc::clone(hit);
        }
        let shaped = Arc::new(shape());
        let mut entries = self.entries.lock();
        // Dropping the whole map rather than one entry: the runs that repeat are the ones the
        // last few passes painted, so what an eviction order would keep is what a fresh map
        // fills itself with anyway, and there is no per-entry bookkeeping to pay for it.
        if entries.len() >= MAX_ENTRIES {
            entries.clear();
        }
        entries.insert(key, Arc::clone(&shaped));
        shaped
    }

    /// Number of shaped runs held, for tests and diagnostics.
    pub fn len(&self) -> usize {
        self.entries.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn style(family: &str, size: f32) -> TextStyle {
        TextStyle::new(family, size)
    }

    #[test]
    fn shapes_once_per_text_and_style() {
        let cache = ShapeCache::new();
        let calls = AtomicUsize::new(0);
        let shape = || {
            calls.fetch_add(1, Ordering::Relaxed);
            ShapedText::empty()
        };

        cache.get_or_shape("hello", &style("serif", 16.0), shape);
        cache.get_or_shape("hello", &style("serif", 16.0), shape);
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn a_different_style_is_a_different_entry() {
        let cache = ShapeCache::new();
        let calls = AtomicUsize::new(0);
        let shape = || {
            calls.fetch_add(1, Ordering::Relaxed);
            ShapedText::empty()
        };

        cache.get_or_shape("hello", &style("serif", 16.0), shape);
        cache.get_or_shape("hello", &style("serif", 17.0), shape);
        cache.get_or_shape("hello", &style("sans-serif", 16.0), shape);
        let mut wrapped = style("serif", 16.0);
        wrapped.max_width = Some(100.0);
        cache.get_or_shape("hello", &wrapped, shape);
        assert_eq!(calls.load(Ordering::Relaxed), 4);
    }

    #[test]
    fn stays_within_its_cap() {
        let cache = ShapeCache::new();
        for i in 0..(MAX_ENTRIES + 10) {
            cache.get_or_shape(&format!("run {i}"), &style("serif", 16.0), ShapedText::empty);
        }
        assert!(cache.len() <= MAX_ENTRIES);
    }
}
