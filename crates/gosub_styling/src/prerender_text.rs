use std::sync::{Arc, Mutex};

use lazy_static::lazy_static;
use rust_fontconfig::{FcFontCache, FcPattern};
use vello::glyph::Glyph;
use vello::kurbo::Affine;
use vello::peniko::{Blob, BrushRef, Font, StyleRef};
use vello::skrifa::instance::Size;
use vello::skrifa::{FontRef, MetadataProvider};
use vello::Scene;

use gosub_html5::node::data::text::TextData;

pub const BACKUP_FONT_NAME: &str = "Roboto";
pub const DEFAULT_FS: f32 = 16.0;

lazy_static! {
    pub static ref FONT_PATH_CACHE: FcFontCache = FcFontCache::build();

    pub static ref FONT_RENDERER_CACHE: Mutex<FontRendererCache> = {
        // we look for the backup font first, then we look for sans-serif, then we just take the first font we find
        // if we can't find any fonts, we panic
        let mut pattern = FcPattern {
            name: Some(BACKUP_FONT_NAME.to_string()),
            ..Default::default()
        };

        let font_path = FONT_PATH_CACHE.query(&pattern).unwrap_or_else(|| {
            pattern = FcPattern {
                name: Some("sans-serif".to_string()),
                ..Default::default()
            };

            FONT_PATH_CACHE.query(&pattern).unwrap_or_else(|| {
                FONT_PATH_CACHE.query_all(&Default::default()).first().unwrap_or_else(|| {
                    panic!("No fonts found")
                })
            })
        });

        //TODO: remove expect here and use a different query
        let font_bytes = std::fs::read(&font_path.path).expect("Failed to read font file");
        let font = Font::new(Blob::new(Arc::new(font_bytes)), 0);

        let backup = TextRenderer {
            pattern,
            font,
            sizing: Vec::new(),
        };

        Mutex::new(FontRendererCache::new(backup))
    };
}

pub struct FontRendererCache {
    renderers: Vec<TextRenderer>,
    pub backup: TextRenderer,
}

enum Index {
    Some(usize),
    Backup,
}

impl Index {
    fn is_backup(&self) -> bool {
        matches!(self, Self::Backup)
    }
}

impl From<Option<usize>> for Index {
    fn from(index: Option<usize>) -> Self {
        match index {
            Some(index) => Self::Some(index),
            None => Self::Backup,
        }
    }
}

enum IndexNoBackup {
    None,
    Some(usize),
    Insert(String),
}

impl IndexNoBackup {
    fn is_none(&self) -> bool {
        matches!(self, Self::None)
    }
}

impl From<Option<usize>> for IndexNoBackup {
    fn from(index: Option<usize>) -> Self {
        match index {
            Some(index) => Self::Some(index),
            None => Self::None,
        }
    }
}

impl FontRendererCache {
    fn new(backup: TextRenderer) -> Self {
        Self {
            renderers: Vec::new(),
            backup,
        }
    }

    fn query_no_backup(&mut self, pattern: FcPattern) -> IndexNoBackup {
        let index: IndexNoBackup = self
            .renderers
            .iter()
            .position(|r| r.pattern == pattern)
            .into();

        if index.is_none() {
            let Some(font_path) = FONT_PATH_CACHE.query(&pattern) else {
                return IndexNoBackup::None;
            };

            return IndexNoBackup::Insert(font_path.path.clone());
        }

        index
    }

    pub fn query(&mut self, pattern: FcPattern) -> &mut TextRenderer {
        if self.backup.pattern == pattern {
            return &mut self.backup;
        }

        // we need to do this with an index value because of https://github.com/rust-lang/rust/issues/21906
        let mut index: Index = self
            .renderers
            .iter()
            .position(|r| r.pattern == pattern)
            .into();

        if index.is_backup() {
            let Some(font_path) = FONT_PATH_CACHE.query(&pattern) else {
                return &mut self.backup;
            };

            let Ok(font_bytes) = std::fs::read(&font_path.path) else {
                return &mut self.backup;
            };

            let font = Font::new(Blob::new(Arc::new(font_bytes)), 0);

            let r = TextRenderer {
                pattern,
                font,
                sizing: Vec::new(),
            };

            self.renderers.push(r);
            index = Index::Some(self.renderers.len() - 1);
        }

        match index {
            Index::Some(index) => &mut self.renderers[index],
            Index::Backup => &mut self.backup,
        }
    }

    pub fn query_ff(&mut self, font_family: Vec<String>) -> &mut TextRenderer {
        let mut renderer = IndexNoBackup::None;
        for f in font_family {
            let pattern = FcPattern {
                name: Some(f),
                ..Default::default()
            };

            let rend = self.query_no_backup(pattern);

            match rend {
                IndexNoBackup::Some(index) => {
                    return &mut self.renderers[index];
                }
                IndexNoBackup::Insert(path) => {
                    renderer = IndexNoBackup::Insert(path);
                }
                IndexNoBackup::None => {}
            }
        }

        match renderer {
            IndexNoBackup::Some(index) => &mut self.renderers[index], //unreachable, but we handle it just in case
            IndexNoBackup::Insert(path) => {
                let font_bytes = std::fs::read(&path).expect("Failed to read font file");
                let font = Font::new(Blob::new(Arc::new(font_bytes)), 0);

                let r = TextRenderer {
                    pattern: FcPattern {
                        name: Some(path),
                        ..Default::default()
                    },
                    font,
                    sizing: Vec::new(),
                };

                let idx = self.renderers.len();
                self.renderers.push(r);
                &mut self.renderers[idx]
            }
            IndexNoBackup::None => &mut self.backup,
        }
    }
}

#[derive(Clone)]
pub struct TextRenderer {
    pattern: FcPattern,
    pub font: Font,
    sizing: Vec<FontSizing>,
}

#[derive(Clone)]
pub struct FontSizing {
    pub font_size: f32,
    pub line_height: f32,
}

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
        let font_ref =
            to_font_ref(&renderer.font).ok_or_else(|| anyhow::anyhow!("Failed to get font ref"))?;

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
            font: renderer.font.clone(),
        })
    }

    pub fn show<'a>(
        &self,
        scene: &mut Scene,
        brush: impl Into<BrushRef<'a>>,
        transform: Affine,
        style: impl Into<StyleRef<'a>>,
        glyph_transform: Option<Affine>,
    ) {
        let brush = brush.into();
        let style = style.into();

        scene
            .draw_glyphs(&self.font)
            .font_size(self.font_size)
            .transform(transform)
            .glyph_transform(glyph_transform)
            .brush(brush)
            .draw(style, self.glyphs.iter().copied());
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
