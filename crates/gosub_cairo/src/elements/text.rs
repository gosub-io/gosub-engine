use crate::CairoBackend;
use cairo::{freetype, FontFace};
use std::borrow::Borrow;
use std::cell::LazyCell;
use std::rc::Rc;
use std::sync::Arc;

use crate::elements::brush::GsBrush;
use crate::elements::color::GsColor;
use cairo::freetype::{Face, Library};
use gosub_interface::font::FontBlob;
use gosub_interface::layout::{Decoration, TextLayout};
use gosub_interface::render_backend::{RenderText, Text as TText};
use gosub_shared::font::{Glyph, GlyphID};
use gosub_shared::geo::{NormalizedCoord, Point, FP};
use log::warn;

thread_local! {
    static LIB_FONT_FACE: LazyCell<Library> = LazyCell::new(|| {
        Library::init().expect("Failed to initialize FreeType")
    });
}

#[allow(unused)]
#[derive(Clone, Debug)]
pub struct GsText {
    // List of positioned glyphs with font info
    glyphs: Vec<Glyph>,
    // Font we need to display
    font_data: FontBlob,
    // Font size
    fs: FP,
    // List of coordinates for each glyph (?)
    coords: Vec<NormalizedCoord>,
    // Text decoration (strike-through, underline, etc.)
    decoration: Decoration,
    // offset in the element
    offset: Point,
}

impl TText for GsText {
    // fn new<TL: TextLayout>(layout: &TL) -> Self {
    fn new(layout: &impl TextLayout) -> Self {
        let glyphs = layout
            .glyphs()
            .iter()
            .map(|g| Glyph {
                id: g.id as GlyphID,
                x: g.x,
                y: g.y,
            })
            .collect();

        Self {
            glyphs,
            font_data: layout.font_data().clone(),
            fs: layout.font_size(),
            coords: layout.coords().to_vec(),
            decoration: layout.decorations().clone(),
            offset: layout.offset(),
        }
    }
}

impl GsText {
    pub(crate) fn render(obj: &RenderText<CairoBackend>, cr: &cairo::Context) {
        let base_x = obj.rect.x;
        let base_y = obj.rect.y;
        cr.move_to(base_x, base_y);

        for text in &obj.text {
            let Ok(font_face) = create_memory_font_face(&text.font_data) else {
                warn!("Could not convert memory face");
                continue;
            };
            cr.set_font_face(&font_face);

            GsBrush::render(&obj.brush, cr);
            cr.move_to(base_x + f64::from(text.offset.x), base_y + f64::from(text.offset.y));
            cr.set_font_size(text.fs.into());

            // Convert glyphs that are in parley / taffy format to cairo glyphs. Also make sure we
            // offset the glyphs by the base_x and base_y.
            let mut cairo_glyphs = vec![];
            for glyph in &text.glyphs {
                let cairo_glyph = cairo::Glyph::new(
                    u64::from(glyph.id),
                    base_x + f64::from(glyph.x) + f64::from(text.offset.x),
                    base_y + f64::from(glyph.y) + f64::from(text.offset.y),
                );
                cairo_glyphs.push(cairo_glyph);
            }

            _ = cr.show_glyphs(&cairo_glyphs);

            // Set decoration (underline, overline, line-through)
            {
                let decoration = &text.decoration;
                let _stroke = kurbo::Stroke::new(f64::from(decoration.width));

                let c = decoration.color;
                let brush = GsBrush::solid(GsColor::rgba32(c.0, c.1, c.2, 1.0));
                GsBrush::render(&brush, cr);

                let offset = f64::from(decoration.x_offset);
                if decoration.underline {
                    let y = base_y + f64::from(decoration.underline_offset) + obj.rect.height;

                    cr.move_to(base_x + offset, y);
                    cr.line_to(base_x + obj.rect.width, y);
                    _ = cr.stroke();
                }
                if decoration.overline {
                    let y = base_y - obj.rect.height;

                    cr.move_to(base_x + offset, y);
                    cr.line_to(base_x + obj.rect.width, y);
                    _ = cr.stroke();
                }

                if decoration.line_through {
                    let y = base_y + obj.rect.height / 2.0;

                    cr.move_to(base_x + offset, y);
                    cr.line_to(base_x + obj.rect.width, y);
                    _ = cr.stroke();
                }
            }
        }
    }
}

#[derive(Clone)]
struct BlobWrapper(Arc<dyn AsRef<[u8]>>);

impl Borrow<[u8]> for BlobWrapper {
    fn borrow(&self) -> &[u8] {
        self.0.as_ref().as_ref()
    }
}

/// Creates a cairo font-face from the font data (blob of raw fontdata). We do this by converting
/// the blob into an in-memory freetype face and then into a cairo font face.
fn create_memory_font_face(font: &FontBlob) -> Result<FontFace, cairo::Error> {
    static FT_FACE_KEY: cairo::UserDataKey<Face<BlobWrapper>> = cairo::UserDataKey::new();

    // Create an in-memory font face from the font data
    let face = LIB_FONT_FACE.with(|lib| {
        lib.new_memory_face2(BlobWrapper(font.data.clone()), font.index as isize)
            .expect("Failed to create memory face")
    });
    let mut face = face.clone();

    // SAFETY: The user data entry keeps `freetype::face::Face` alive
    // until the FontFace is dropped.
    let font_face = unsafe {
        FontFace::from_raw_full(cairo::ffi::cairo_ft_font_face_create_for_ft_face(
            (face.raw_mut() as freetype::ffi::FT_Face).cast(),
            0,
        ))
    };
    font_face.set_user_data(&FT_FACE_KEY, Rc::new(face))?;
    let status = unsafe { cairo::ffi::cairo_font_face_status(font_face.to_raw_none()) };
    match status {
        cairo::ffi::STATUS_SUCCESS => {}
        err => return Err(err.into()),
    }

    Ok(font_face)
}
