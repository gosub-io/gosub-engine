/// RGBA color used for drawing commands.
///
/// Channels are represented as `f32` in the range `0.0 ..= 1.0`.
#[derive(Debug, Clone, Copy)]
pub struct Color {
    /// Red channel
    pub r: f32,
    /// Green channel
    pub g: f32,
    /// Blue channel
    pub b: f32,
    /// Alpha channel (opacity)
    pub a: f32,
}

impl Into<[f32; 4]> for Color {
    fn into(self) -> [f32; 4] {
        [self.r, self.g, self.b, self.a]
    }
}

impl Into<[u8; 4]> for Color {
    fn into(self) -> [u8; 4] {
        [self.r_u8(), self.g_u8(), self.b_u8(), self.a_u8()]
    }
}

impl Color {
    pub const BLACK: Color = Color {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 1.0,
    };
    pub const WHITE: Color = Color {
        r: 1.0,
        g: 1.0,
        b: 1.0,
        a: 1.0,
    };
    pub const TRANSPARENT: Color = Color {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 0.0,
    };

    pub const RED: Color = Color {
        r: 1.0,
        g: 0.0,
        b: 0.0,
        a: 1.0,
    };
    pub const GREEN: Color = Color {
        r: 0.0,
        g: 1.0,
        b: 0.0,
        a: 1.0,
    };
    pub const BLUE: Color = Color {
        r: 0.0,
        g: 0.0,
        b: 1.0,
        a: 1.0,
    };
    pub const YELLOW: Color = Color {
        r: 1.0,
        g: 1.0,
        b: 0.0,
        a: 1.0,
    };
    pub const CYAN: Color = Color {
        r: 0.0,
        g: 1.0,
        b: 1.0,
        a: 1.0,
    };
    pub const MAGENTA: Color = Color {
        r: 1.0,
        g: 0.0,
        b: 1.0,
        a: 1.0,
    };

    /// Creates a new color from `f32` channel values in the range `0.0 ..= 1.0`.
    pub fn new(r: f32, g: f32, b: f32, a: f32) -> Color {
        Color { r, g, b, a }
    }

    /// Creates a new color from `u8` channel values in the range `0 ..= 255`.
    pub fn from_u8(r: u8, g: u8, b: u8, a: u8) -> Color {
        Color {
            r: r as f32 / 255.0,
            g: g as f32 / 255.0,
            b: b as f32 / 255.0,
            a: a as f32 / 255.0,
        }
    }

    /// Returns the red channel as an `u8` (0–255).
    fn r_u8(&self) -> u8 {
        (self.r * 255.0) as u8
    }

    /// Returns the green channel as an `u8` (0–255).
    fn g_u8(&self) -> u8 {
        (self.g * 255.0) as u8
    }

    /// Returns the blue channel as an `u8` (0–255).
    fn b_u8(&self) -> u8 {
        (self.b * 255.0) as u8
    }

    /// Returns the alpha channel as an `u8` (0–255).
    fn a_u8(&self) -> u8 {
        (self.a * 255.0) as u8
    }
}

/// A single display item representing a drawing command.
///
/// These commands are appended to a [`RenderList`] and later processed
/// by the render backend.
///
/// Variants:
/// - [`DisplayItem::Clear`] — clear the entire surface to a color.
/// - [`DisplayItem::Rect`] — draw a solid rectangle.
/// - [`DisplayItem::TextRun`] — draw a run of text at a position.
#[derive(Clone, Debug)]
pub enum DisplayItem {
    /// Clear the entire surface with the given color.
    Clear {
        /// The color to clear the surface with.
        color: Color,
    },

    /// Draw a filled rectangle at `(x, y)` with width `w` and height `h`.
    Rect {
        /// The x-coordinate of the rectangle's top-left corner.
        x: f32,
        /// The y-coordinate of the rectangle's top-left corner.
        y: f32,
        /// The width of the rectangle.
        w: f32,
        /// The height of the rectangle.
        h: f32,
        /// The color to fill the rectangle with.
        color: Color,
    },

    /// Draw a 1px stroked rectangle outline (no fill) at `(x, y)` with width `w` and height `h`.
    Outline {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        color: Color,
    },

    /// Draw a text run at `(x, y)` with font size `size`.
    TextRun {
        /// The x-coordinate where the text starts.
        x: f32,
        /// The y-coordinate where the text starts.
        y: f32,
        /// The text to render.
        text: String,
        /// The font size to use for the text.
        size: f32,
        /// The color to render the text with.
        color: Color,
        /// Optional maximum width for text wrapping (in pixels).
        max_width: Option<f32>,
    },
}

/// Render list and display items.
///
/// This module defines a lightweight, immediate-style render list
/// consisting of [`DisplayItem`] commands. It acts as a temporary
/// system for testing and prototyping before the full render pipeline
/// is integrated.
///
/// The core type is [`RenderList`], which collects a sequence of
/// display items such as rectangles, text runs, or clears, and can
/// later be consumed by a compositor or renderer.
///
/// # Example
///
/// ```rust
/// use gosub_engine::render::{RenderList, DisplayItem, Color};
///
/// let mut list = RenderList::new();
///
/// // Clear background
/// list.add_command(DisplayItem::Clear { color: Color::from_u8(0, 0, 0, 255) });
///
/// // Draw a white rectangle
/// list.add_command(DisplayItem::Rect {
///     x: 10.0,
///     y: 20.0,
///     w: 100.0,
///     h: 50.0,
///     color: Color::from_u8(255, 255, 255, 255),
/// });
/// ```
#[derive(Clone, Debug, Default)]
pub struct RenderList {
    /// Sequence of drawing commands to execute.
    pub items: Vec<DisplayItem>,
}

impl RenderList {
    /// Creates a new, empty render list.
    pub fn new() -> Self {
        RenderList { items: Vec::new() }
    }

    /// Adds a new display item (drawing command) to the list.
    pub fn add_command(&mut self, command: DisplayItem) {
        self.items.push(command);
    }

    /// Clears all display items from the list.
    pub fn clear(&mut self) {
        self.items.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx_eq4(a: [f32; 4], b: [f32; 4], eps: f32) {
        for i in 0..4 {
            assert!(
                (a[i] - b[i]).abs() <= eps,
                "index {i}: left={} right={} (eps={eps})",
                a[i],
                b[i]
            );
        }
    }

    #[test]
    fn color_new_and_into_f32() {
        let c = Color::new(0.25, 0.5, 0.75, 1.0);
        let arr: [f32; 4] = c.into();
        approx_eq4(arr, [0.25, 0.5, 0.75, 1.0], 1e-6);
    }

    #[test]
    fn color_from_u8_and_into_f32() {
        // 255 maps to 1.0, 0 to 0.0, mid-values map to v/255
        let c = Color::from_u8(255, 0, 128, 64);
        let arr: [f32; 4] = c.into();
        // 128/255 ≈ 0.5019608, 64/255 ≈ 0.2509804
        approx_eq4(arr, [1.0, 0.0, 128.0 / 255.0, 64.0 / 255.0], 1e-6);
    }

    #[test]
    fn color_into_u8_roundtrip_like() {
        // Build via from_u8 -> to [u8;4]; should get the same bytes back
        let c = Color::from_u8(12, 34, 56, 78);
        let bytes: [u8; 4] = c.into();
        assert_eq!(bytes, [12, 34, 56, 78]);

        // Also check the channel helpers
        let c = Color::from_u8(200, 150, 100, 50);
        assert_eq!(c.r_u8(), 200);
        assert_eq!(c.g_u8(), 150);
        assert_eq!(c.b_u8(), 100);
        assert_eq!(c.a_u8(), 50);
    }

    #[test]
    fn color_copy_clone() {
        // Ensure Copy + Clone behave as expected
        let c = Color::from_u8(10, 20, 30, 40);
        let c2 = c; // Copy
        let c3 = c2.clone(); // Clone
        let a: [u8; 4] = c3.into();
        assert_eq!(a, [10, 20, 30, 40]);
    }

    #[test]
    fn displayitem_clear_debug() {
        let di = DisplayItem::Clear {
            color: Color::from_u8(0, 0, 0, 255),
        };
        let dbg = format!("{di:?}");
        assert!(dbg.contains("Clear"));
        assert!(dbg.contains("Color"));
    }

    #[test]
    fn displayitem_rect_fields() {
        let di = DisplayItem::Rect {
            x: 10.0,
            y: 20.0,
            w: 100.0,
            h: 50.0,
            color: Color::from_u8(255, 255, 255, 255),
        };

        match di {
            DisplayItem::Rect { x, y, w, h, color } => {
                assert_eq!(x, 10.0);
                assert_eq!(y, 20.0);
                assert_eq!(w, 100.0);
                assert_eq!(h, 50.0);
                let bytes: [u8; 4] = color.into();
                assert_eq!(bytes, [255, 255, 255, 255]);
            }
            _ => panic!("Expected Rect variant"),
        }
    }

    #[test]
    fn displayitem_textrun_with_and_without_wrap() {
        let black = Color::from_u8(0, 0, 0, 255);

        let no_wrap = DisplayItem::TextRun {
            x: 5.0,
            y: 7.0,
            text: "Hello".to_string(),
            size: 14.0,
            color: black,
            max_width: None,
        };

        if let DisplayItem::TextRun {
            x,
            y,
            text,
            size,
            color,
            max_width,
        } = no_wrap
        {
            assert_eq!(x, 5.0);
            assert_eq!(y, 7.0);
            assert_eq!(text, "Hello");
            assert_eq!(size, 14.0);
            let b: [u8; 4] = color.into();
            assert_eq!(b, [0, 0, 0, 255]);
            assert!(max_width.is_none());
        } else {
            panic!("Expected TextRun");
        }

        let with_wrap = DisplayItem::TextRun {
            x: 0.0,
            y: 0.0,
            text: "Wrapped".into(),
            size: 12.0,
            color: black,
            max_width: Some(200.0),
        };

        if let DisplayItem::TextRun { max_width, .. } = with_wrap {
            assert_eq!(max_width, Some(200.0));
        } else {
            panic!("Expected TextRun");
        }
    }

    #[test]
    fn renderlist_new_is_empty() {
        let rl = RenderList::new();
        assert!(rl.items.is_empty());
        assert_eq!(rl.items.len(), 0);
    }

    #[test]
    fn renderlist_add_command_keeps_order() {
        let mut rl = RenderList::new();

        rl.add_command(DisplayItem::Clear {
            color: Color::from_u8(10, 20, 30, 255),
        });
        rl.add_command(DisplayItem::Rect {
            x: 1.0,
            y: 2.0,
            w: 3.0,
            h: 4.0,
            color: Color::from_u8(255, 0, 0, 255),
        });
        rl.add_command(DisplayItem::TextRun {
            x: 100.0,
            y: 200.0,
            text: "abc".into(),
            size: 16.0,
            color: Color::from_u8(255, 255, 255, 255),
            max_width: Some(300.0),
        });

        assert_eq!(rl.items.len(), 3);

        match &rl.items[0] {
            DisplayItem::Clear { color } => {
                let b: [u8; 4] = (*color).into();
                assert_eq!(b, [10, 20, 30, 255]);
            }
            _ => panic!("Expected Clear at index 0"),
        }

        match &rl.items[1] {
            DisplayItem::Rect { x, y, w, h, .. } => {
                assert_eq!((*x, *y, *w, *h), (1.0, 2.0, 3.0, 4.0));
            }
            _ => panic!("Expected Rect at index 1"),
        }

        match &rl.items[2] {
            DisplayItem::TextRun {
                text, size, max_width, ..
            } => {
                assert_eq!(text, "abc");
                assert_eq!(*size, 16.0);
                assert_eq!(*max_width, Some(300.0));
            }
            _ => panic!("Expected TextRun at index 2"),
        }
    }

    #[test]
    fn renderlist_clear_removes_all() {
        let mut rl = RenderList::new();
        rl.add_command(DisplayItem::Clear {
            color: Color::from_u8(0, 0, 0, 255),
        });
        rl.add_command(DisplayItem::Clear {
            color: Color::from_u8(255, 255, 255, 255),
        });

        assert_eq!(rl.items.len(), 2);
        rl.clear();
        assert!(rl.items.is_empty());
    }
}
