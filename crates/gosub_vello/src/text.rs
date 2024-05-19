use std::ops::Deref;
use vello::glyph::Glyph;
use vello::kurbo::Affine;
use vello::peniko::{Blob, BrushRef, Fill, Font, StyleRef};
use vello::skrifa::{instance::Size as FSize, FontRef, MetadataProvider};

use gosub_render_backend::{PreRenderText as TPreRenderText, RenderText, Size, Text as TText, FP};
use gosub_typeface::{BACKUP_FONT, DEFAULT_LH, FONT_RENDERER_CACHE};

use crate::VelloBackend;

pub struct Text {
    glyphs: Vec<Glyph>,
    font: Vec<Font>,
    fs: FP,
}

pub struct PreRenderText {
    text: String,
    fs: FP,
    font: Vec<Font>,
    line_height: FP,
    size: Option<Size>,
    glyphs: Option<Vec<Glyph>>,
}

impl TText<VelloBackend> for Text {
    fn new(pre: &mut PreRenderText, backend: &VelloBackend) -> Self {
        if pre.glyphs.is_none() {
            pre.prerender(backend);
        }

        Text {
            glyphs: pre.glyphs.clone().unwrap_or_default(),
            font: pre.font.clone(),
            fs: pre.fs,
        }
    }
}

fn get_fonts_from_family(font_families: Option<Vec<String>>) -> Vec<Font> {
    let mut fonts = Vec::with_capacity(font_families.as_ref().map(|f| f.len()).unwrap_or(1));

    if let Ok(mut cache) = FONT_RENDERER_CACHE.lock() {
        if let Some(ff) = font_families {
            let font = cache.query_all_shared(ff);
            for (i, f) in font.into_iter().enumerate() {
                fonts.push(Font::new(Blob::new(f), i as u32));
            }
        }
    } else {
        fonts.push(Font::new(Blob::new(BACKUP_FONT.data.clone()), 0));
    }

    fonts
}

impl TPreRenderText<VelloBackend> for PreRenderText {
    fn new(text: String, font: Option<Vec<String>>, size: FP) -> Self {
        let font = get_fonts_from_family(font);

        PreRenderText {
            text,
            font,
            line_height: DEFAULT_LH,
            size: None,
            fs: size,
            glyphs: None,
        }
    }

    fn with_lh(text: String, font: Option<Vec<String>>, size: FP, line_height: FP) -> Self {
        let font = get_fonts_from_family(font);

        PreRenderText {
            text,
            font,
            line_height,
            size: None,
            fs: size,
            glyphs: None,
        }
    }

    fn prerender(&mut self, backend: &VelloBackend) -> Size {
        let font_ref = to_font_ref(&self.font[0]).unwrap();

        let axes = font_ref.axes();
        let char_map = font_ref.charmap();
        let fs = FSize::new(self.fs);
        let variations: &[(&str, f32)] = &[]; // if we have more than an empty slice here we need to change the rendering to the scene
        let var_loc = axes.location(variations.iter().copied());
        let glyph_metrics = font_ref.glyph_metrics(fs, &var_loc);
        let metrics = font_ref.metrics(fs, &var_loc);
        // let line_height = metrics.ascent - metrics.descent + metrics.leading;

        let mut width: f32 = 0.0;
        let mut pen_x: f32 = 0.0;

        let glyphs = self
            .text
            .chars()
            .filter_map(|c| {
                if c == '\n' {
                    return None;
                }

                let gid = char_map.map(c).unwrap_or_default(); //TODO: here we need to use the next font if the glyph is not found
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

        self.glyphs = Some(glyphs);

        Size {
            width,
            height: self.line_height,
        }
    }

    fn value(&self) -> &str {
        self.text.as_ref()
    }

    fn fs(&self) -> FP {
        self.fs
    }
}

impl Text {
    fn show(vello: &mut VelloBackend, render: RenderText<VelloBackend>) {
        let brush = render.brush.0;
        let style: StyleRef = Fill::NonZero.into();

        let transform = render.transform.map(|t| t.0).unwrap_or(Affine::IDENTITY);
        let brush_transform = render.brush_transform.map(|t| t.0);

        vello
            .scene
            .draw_glyphs(&render.text.font[0])
            .font_size(render.text.fs)
            .transform(transform)
            .glyph_transform(brush_transform)
            .brush(&brush)
            .draw(style, render.text.glyphs.iter().copied());
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
