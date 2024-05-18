use std::sync::{Arc, Mutex};

use lazy_static::lazy_static;
#[cfg(not(target_arch = "wasm32"))]
use vello::glyph::Glyph;
use vello::kurbo::Affine;
use vello::peniko::{Blob, BrushRef, Font, StyleRef};
use vello::skrifa::instance::Size;
use vello::skrifa::{FontRef, MetadataProvider};
use vello::Scene;

use gosub_html5::node::data::text::TextData;
use gosub_render_backend::RenderBackend;
use gosub_typeface::{FontSizing, TextRenderer, FONT_RENDERER_CACHE};

#[derive(Debug)]
pub struct PrerenderText {
    pub text: String,
    pub width: f32,
    pub height: f32,
    pub line_height: f32,
    pub font_size: f32,
    pub glyphs: Vec<Glyph>,
    pub font: Font,
}

#[allow(clippy::from_over_into)]
impl Into<TextData> for PrerenderText {
    fn into(self) -> TextData {
        TextData { value: self.text }
    }
}

#[allow(clippy::from_over_into)]
impl Into<TextData> for &PrerenderText {
    fn into(self) -> TextData {
        TextData {
            value: self.text.clone(),
        }
    }
}

impl PrerenderText {
    pub fn new(text: String, font_size: f32, font_family: Vec<String>) -> anyhow::Result<Self> {
        let mut renderers_cache = FONT_RENDERER_CACHE
            .lock()
            .map_err(|_| anyhow::anyhow!("Failed to lock font renderer cache"))?;
        let renderer = renderers_cache.query_ff(font_family);

        Self::with_renderer(text, font_size, renderer)
    }

    pub fn with_renderer(
        text: String,
        font_size: f32,
        renderer: &mut TextRenderer,
    ) -> anyhow::Result<Self> {
        let font = Font::new(Blob::new(renderer.font.data), 0);
        let font_ref =
            to_font_ref(&font).ok_or_else(|| anyhow::anyhow!("Failed to get font ref"))?;

        let axes = font_ref.axes();
        let char_map = font_ref.charmap();
        let fs = Size::new(font_size);
        let variations: &[(&str, f32)] = &[]; // if we have more than an empty slice here we need to change the rendering to the scene
        let var_loc = axes.location(variations.iter().copied());
        let glyph_metrics = font_ref.glyph_metrics(fs, &var_loc);
        let metrics = font_ref.metrics(fs, &var_loc);
        let line_height = metrics.ascent - metrics.descent + metrics.leading;

        let sizing = FontSizing {
            font_size,
            line_height,
        };

        let mut width: f32 = 0.0;
        let mut pen_x: f32 = 0.0;

        let glyphs = text
            .chars()
            .filter_map(|c| {
                if c == '\n' {
                    return None;
                }

                let gid = char_map.map(c).unwrap_or_default();
                let advance = glyph_metrics.advance_width(gid).unwrap_or_default();
                let x = pen_x;
                pen_x += advance;

                Some(Glyph {
                    id: gid.to_u16() as u32,
                    x,
                    y: 0.0,
                })
            })
            .collect();

        width = width.max(pen_x);

        renderer.sizing.push(sizing);

        Ok(Self {
            text,
            width,
            height: line_height,
            line_height,
            font_size,
            glyphs,
            font,
        })
    }

    pub fn show<'a, B: RenderBackend>(
        &self,
        scene: &mut B,
        brush: impl Into<BrushRef<'a>>,
        transform: B::Transform,
        style: impl Into<StyleRef<'a>>,
        glyph_transform: Option<B::Transform>,
    ) {
        let brush = brush.into();
        let style = style.into();

        let _ = (scene, transform, glyph_transform, brush, style);

        todo!()

        // scene
        //     .draw_glyphs(&self.font)
        //     .font_size(self.font_size)
        //     .transform(transform)
        //     .glyph_transform(glyph_transform)
        //     .brush(brush)
        //     .draw(style, self.glyphs.iter().copied());
    }
}

fn to_font_ref(font: &Font) -> Option<FontRef<'_>> {
    use vello::skrifa::raw::FileRef;
    let file_ref = FileRef::new(font.data.as_ref()).ok()?;
    match file_ref {
        FileRef::Font(font) => Some(font),
        FileRef::Collection(collection) => collection.get(font.index).ok(),
    }
}
