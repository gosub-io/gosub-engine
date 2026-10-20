//! The visual viewport: the part of the layout viewport that is on screen, and how large it is
//! drawn.
//!
//! The layout viewport is what the page is laid out in ([`PageScale`]); it scrolls over the page
//! as it always has. A pinch zoom shows less of it, larger: the visual viewport is then smaller
//! than the layout viewport and sits at an offset inside it. A pan moves the visual viewport
//! first and the layout viewport only once the visual one reaches its edge, so `position: fixed`
//! content, which belongs to the layout viewport, stays put while a zoomed-in reader looks
//! around it. This is the model of the CSSOM View visual viewport and of the mobile browsers.
//!
//! Screen coordinates here are the units the embedder sets the viewport in: CSS px of the screen
//! at a scale of 1. Everything else is CSS px of the page.

use crate::engine::viewport_meta::PageScale;

/// Where the visual viewport is and how large it is drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VisualViewport {
    /// The screen it is drawn on.
    screen: (f64, f64),
    /// The layout viewport it sits in, and how that was made.
    page: PageScale,
    /// Screen px per page px.
    scale: f64,
    /// The top-left inside the layout viewport.
    offset: (f64, f64),
}

impl VisualViewport {
    /// Unzoomed: the whole layout viewport, at the scale that fits it to the screen.
    pub fn new(screen_width: u32, screen_height: u32, page: PageScale) -> Self {
        Self {
            screen: (f64::from(screen_width), f64::from(screen_height)),
            page,
            scale: page.scale,
            offset: (0.0, 0.0),
        }
    }

    /// The screen and the layout viewport it shows, to tell whether either changed.
    pub fn shows(&self, screen_width: u32, screen_height: u32, page: &PageScale) -> bool {
        self.screen == (f64::from(screen_width), f64::from(screen_height)) && self.page == *page
    }

    /// Screen px per page px.
    pub fn scale(&self) -> f64 {
        self.scale
    }

    /// The top-left inside the layout viewport, page px.
    pub fn offset(&self) -> (f64, f64) {
        self.offset
    }

    /// The size, page px.
    pub fn size(&self) -> (f64, f64) {
        (self.screen.0 / self.scale, self.screen.1 / self.scale)
    }

    /// Whether this is all of the layout viewport, as it is until the user zooms.
    pub fn unzoomed(&self) -> bool {
        self.scale == self.page.scale && self.offset == (0.0, 0.0)
    }

    /// Back to all of the layout viewport, as a new page starts.
    pub fn reset(&mut self) {
        self.scale = self.page.scale;
        self.offset = (0.0, 0.0);
    }

    /// A screen point as a layout-viewport point, the coordinates hit testing takes.
    pub fn layout_point(&self, x: f64, y: f64) -> (f64, f64) {
        (self.offset.0 + x / self.scale, self.offset.1 + y / self.scale)
    }

    /// Move by a delta in page px. Returns what is left for the layout viewport to scroll, once
    /// the visual viewport has reached its edge.
    pub fn pan(&mut self, dx: f64, dy: f64) -> (f64, f64) {
        // A NaN offset would stick: no clamp takes it out again.
        if !dx.is_finite() || !dy.is_finite() {
            return (0.0, 0.0);
        }
        let (max_x, max_y) = self.max_offset();
        let x = self.offset.0 + dx;
        let y = self.offset.1 + dy;
        self.offset = (x.clamp(0.0, max_x), y.clamp(0.0, max_y));
        (x - self.offset.0, y - self.offset.1)
    }

    /// Zoom by `factor` about the screen point `focus`, keeping the page point under it there.
    ///
    /// `scroll` is the layout scroll and `max_scroll` how far it can go. Returns the layout scroll
    /// the zoom needs, which moves only when the visual viewport cannot reach where it has to be
    /// from inside the layout viewport; `None` when the page may not be zoomed.
    pub fn pinch(
        &mut self,
        focus: (f64, f64),
        factor: f64,
        scroll: (f64, f64),
        max_scroll: (f64, f64),
    ) -> Option<(f64, f64)> {
        if !self.page.user_scalable
            || !factor.is_finite()
            || factor <= 0.0
            || !focus.0.is_finite()
            || !focus.1.is_finite()
        {
            return None;
        }
        // Never out past the layout viewport: there is nothing to show beside it.
        let min = self.page.scale;
        let max = self.page.max_scale.max(min);
        let scale = (self.scale * factor).clamp(min, max);
        let page_x = scroll.0 + self.offset.0 + focus.0 / self.scale;
        let page_y = scroll.1 + self.offset.1 + focus.1 / self.scale;
        self.scale = scale;
        Some(self.place((page_x - focus.0 / scale, page_y - focus.1 / scale), scroll, max_scroll))
    }

    /// Put the top-left at page point `origin`, moving the layout viewport only as far as the
    /// visual viewport has to go past it. Returns the layout scroll.
    fn place(&mut self, origin: (f64, f64), scroll: (f64, f64), max_scroll: (f64, f64)) -> (f64, f64) {
        let (max_x, max_y) = self.max_offset();
        let axis = |origin: f64, scroll: f64, max_offset: f64, max_scroll: f64| {
            let scroll = scroll
                .clamp(origin - max_offset, origin)
                .clamp(0.0, max_scroll.max(0.0));
            (scroll, (origin - scroll).clamp(0.0, max_offset))
        };
        let (sx, ox) = axis(origin.0, scroll.0, max_x, max_scroll.0);
        let (sy, oy) = axis(origin.1, scroll.1, max_y, max_scroll.1);
        self.offset = (ox, oy);
        (sx, sy)
    }

    /// How far the visual viewport can move inside the layout viewport.
    fn max_offset(&self) -> (f64, f64) {
        // Unzoomed it is the layout viewport, even where the layout height, a whole number of
        // pixels, leaves the screen's height a fraction of a pixel over.
        if self.scale <= self.page.scale {
            return (0.0, 0.0);
        }
        let (w, h) = self.size();
        (
            (f64::from(self.page.layout_width) - w).max(0.0),
            (f64::from(self.page.layout_height) - h).max(0.0),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 360x780 phone showing a page laid out at 980px, scaled to fit.
    fn phone() -> VisualViewport {
        VisualViewport::new(360, 780, PageScale::resolve(360, 780, None))
    }

    #[test]
    fn unzoomed_it_is_the_layout_viewport() {
        let vv = phone();
        assert!(vv.unzoomed());
        let (w, h) = vv.size();
        assert!((w - 980.0).abs() < 1e-9);
        assert!((h - 2123.0).abs() < 1.0);
        // A tap at the screen's right edge lands at the layout viewport's.
        assert!((vv.layout_point(360.0, 0.0).0 - 980.0).abs() < 1e-9);
    }

    #[test]
    fn unzoomed_a_pan_is_all_layout_scroll() {
        let mut vv = phone();
        assert_eq!(vv.pan(0.0, 300.0), (0.0, 300.0));
        assert!(vv.unzoomed());
    }

    #[test]
    fn the_point_under_the_fingers_stays_there() {
        let mut vv = phone();
        let focus = (180.0, 390.0);
        let before = vv.layout_point(focus.0, focus.1);
        let scroll = vv.pinch(focus, 2.0, (0.0, 0.0), (0.0, 5000.0)).unwrap();
        let after = vv.layout_point(focus.0, focus.1);
        assert_eq!(scroll, (0.0, 0.0), "zooming in stays inside the layout viewport");
        assert!((before.0 - after.0).abs() < 1e-9 && (before.1 - after.1).abs() < 1e-9);
        assert!((vv.scale() - 2.0 * 360.0 / 980.0).abs() < 1e-9);
    }

    #[test]
    fn zoom_stays_between_fit_and_the_maximum() {
        let mut vv = phone();
        vv.pinch((0.0, 0.0), 0.5, (0.0, 0.0), (0.0, 0.0));
        assert!(vv.unzoomed(), "nothing to show beside the layout viewport");
        vv.pinch((0.0, 0.0), 1000.0, (0.0, 0.0), (0.0, 0.0));
        assert_eq!(vv.scale(), 5.0);
    }

    #[test]
    fn a_page_that_may_not_be_zoomed_is_not() {
        let meta = crate::engine::viewport_meta::ViewportMeta::parse("width=device-width, user-scalable=no");
        let mut vv = VisualViewport::new(360, 780, PageScale::resolve(360, 780, Some(&meta)));
        assert_eq!(vv.pinch((0.0, 0.0), 2.0, (0.0, 0.0), (0.0, 0.0)), None);
        assert!(vv.unzoomed());
    }

    #[test]
    fn zoomed_in_a_pan_moves_the_visual_viewport_before_the_layout_one() {
        let mut vv = phone();
        vv.pinch((0.0, 0.0), 4.0, (0.0, 0.0), (0.0, 5000.0));
        let (w, _) = vv.size();
        // Right, as far as the layout viewport goes, and 100px past it.
        let (dx, _) = vv.pan(980.0 - w + 100.0, 0.0);
        assert!((vv.offset().0 - (980.0 - w)).abs() < 1e-9);
        assert!((dx - 100.0).abs() < 1e-9);
        // Back left: the visual viewport moves, nothing is left over.
        let (dx, _) = vv.pan(-50.0, 0.0);
        assert_eq!(dx, 0.0);
    }

    #[test]
    fn a_zoom_near_the_bottom_scrolls_the_layout_viewport_only_as_needed() {
        let mut vv = phone();
        let (_, h) = vv.size();
        // The layout viewport scrolled to the end of a 5000px page; zoom out from deep inside.
        let max_scroll = (0.0, 5000.0 - h);
        vv.pinch((180.0, 780.0), 3.0, (0.0, 1000.0), max_scroll).unwrap();
        // Zooming back out from the bottom edge must not leave a gap below the page: the layout
        // scroll goes no further than its maximum, and the visual viewport fills what is left.
        let scroll = vv
            .pinch((180.0, 780.0), 1.0 / 3.0, (0.0, max_scroll.1), max_scroll)
            .unwrap();
        assert!(scroll.1 <= max_scroll.1);
        assert!((vv.scale() - 360.0 / 980.0).abs() < 1e-9);
        assert_eq!(vv.offset().0, 0.0);
    }

    #[test]
    fn input_that_is_not_a_number_leaves_the_viewport_alone() {
        let mut vv = phone();
        vv.pinch((180.0, 390.0), 2.0, (0.0, 0.0), (0.0, 5000.0)).unwrap();
        let (scale, offset) = (vv.scale(), vv.offset());
        assert_eq!(vv.pinch((f64::NAN, 0.0), 2.0, (0.0, 0.0), (0.0, 5000.0)), None);
        assert_eq!(vv.pinch((0.0, f64::INFINITY), 2.0, (0.0, 0.0), (0.0, 5000.0)), None);
        assert_eq!(vv.pan(f64::NAN, 10.0), (0.0, 0.0));
        assert_eq!(vv.pan(0.0, f64::NEG_INFINITY), (0.0, 0.0));
        assert_eq!((vv.scale(), vv.offset()), (scale, offset));
    }
}
