use crate::CairoBackend;
use gosub_interface::layout::{Decoration, TextLayout};
use gosub_interface::render_backend::{RenderText, Text as TText};
use gosub_shared::font::{Glyph, GlyphID};
use gosub_shared::geo::{NormalizedCoord, Point, FP};

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::elements::brush::GsBrush;
use crate::elements::color::GsColor;
use freetype::{Face, Library};
use gosub_shared::ROBOTO_FONT;
use kurbo::Stroke;
use log::info;
use once_cell::sync::Lazy;
use parley::fontique::{FamilyId, SourceKind};
use parley::{FontContext, GenericFamily};

/// Font manager that keeps track of fonts and faces
struct GosubFontContext {
    /// Freetype library. Should be kept alive as long as any face is alive.
    _library: Library,
    /// Font context for parley to find fonts
    font_ctx: FontContext,
    /// Cache of any loaded font faces
    face_cache: HashMap<String, Face>,
    /// Default font face to use when a font cannot be found
    default_face: Face,
}

impl GosubFontContext {
    /// Finds the face for the given family name, or returns the default face if no font is found.
    fn find_face_family(&mut self, family: &str) -> &mut Face {
        info!("Finding face for family: {}", family);

        // See if we already got the face in cache
        if self.face_cache.contains_key(family) {
            info!("Face found in cache");
            return self.face_cache.get_mut(family).expect("Face not found in cache");
        }

        // Parse the family name into a GenericFamily enum
        let gf = GenericFamily::parse(family).unwrap_or(GenericFamily::SansSerif);

        // Find all the fonts for this family
        let fids: Vec<FamilyId> = self.font_ctx.collection.generic_families(gf).collect();
        if fids.is_empty() {
            info!("No family found for family: {}", family);
            return &mut self.default_face;
        }

        // We only use the first font in the family
        match self.font_ctx.collection.family(fids[0]) {
            Some(f) => {
                info!("Face found for family: {:?}", f.fonts());

                // This first font can have multiple fonts (e.g. regular, bold, italic, etc.)
                for font in f.fonts() {
                    match &font.source().kind {
                        SourceKind::Memory(blob) => {
                            info!("Loading font face from memory");
                            let rc = Rc::new(blob.data().to_vec());

                            let face = self._library.new_memory_face(rc, 0).expect("Failed to load font face");
                            self.face_cache.insert(family.to_string(), face);
                        }
                        SourceKind::Path(path) => {
                            info!("Loading font face from path {}", path.to_str().expect("path to string"));

                            let face = self
                                ._library
                                .new_face(path.to_str().expect("path to string"), 0)
                                .expect("Failed to load font face");
                            self.face_cache.insert(family.to_string(), face);
                        }
                    }
                }
            }
            None => {
                info!("No face found for family: {}", family);
            }
        }

        &mut self.default_face
    }
}

thread_local! {
    /// We use a thread-local lazy static to ensure the font context is initialized once per thread
    /// and is dropped when the thread exits. We need this because the FreeType library cannot be dropped
    /// while any faces are still alive, so all is managed within this struct.
    static LIB_FONT_FACE: Lazy<RefCell<GosubFontContext>> = Lazy::new(|| {
        let lib = Library::init().expect("Failed to initialize FreeType");
        let rc = Rc::new(ROBOTO_FONT.to_vec());
        let default_face = lib.new_memory_face(rc, 0).expect("Failed to load font face");

        // The FontContext struct holds the lib, ensuring it lives as long as all loaded faces
        RefCell::new(GosubFontContext {
            _library: lib,
            font_ctx: FontContext::new(),
            face_cache: HashMap::new(),
            default_face,
        })
    });
}

#[allow(unused)]
#[derive(Clone, Debug)]
pub struct GsText {
    // List of glyphs we need to show
    glyphs: Vec<Glyph>,
    // Actual utf-8 text (we don't have this yet)
    text: String,
    // // Font we need to display (we need to have more info, like font familty, weight, etc.)
    // font: peniko::Font,
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
    fn new<TL: TextLayout>(layout: &TL) -> Self {
        // let font = layout.font().clone().into();
        let fs = layout.font_size();

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
            text: String::new(),
            // font,
            fs,
            coords: layout.coords().to_vec(),
            decoration: layout.decorations().clone(),
            offset: layout.offset(),
        }
    }
}

impl GsText {
    pub(crate) fn render(obj: &RenderText<CairoBackend>, cr: &cairo::Context) {
        // let brush = &render.brush;
        // let style: StyleRef = Fill::NonZero.into();
        //
        // let transform = render.transform.map(|t| t).unwrap_or(Transform::IDENTITY);
        // let brush_transform = render.brush_transform.map(|t| t);

        let base_x = obj.rect.x;
        let base_y = obj.rect.y;
        cr.move_to(base_x, base_y);

        // Setup brush for rendering text

        // This should be moved to the GosubFontContext::get_cairo_font_face(family: &str) method)
        let font_face = unsafe {
            LIB_FONT_FACE.with(|ctx_ref| {
                let mut ctx = ctx_ref.borrow_mut();

                let ft_face = ctx.find_face_family("sans-serif");
                let ft_face_ptr = ft_face.raw_mut() as *mut _ as *mut std::ffi::c_void;
                let ff = cairo::ffi::cairo_ft_font_face_create_for_ft_face(ft_face_ptr, 0);
                cairo::FontFace::from_raw_full(ff)
            })
        };
        cr.set_font_face(&font_face);

        for text in &obj.text {
            GsBrush::render(&obj.brush, cr);
            cr.move_to(base_x + text.offset.x as f64, base_y + text.offset.y as f64);
            cr.set_font_size(text.fs.into());

            // Convert glyphs that are in parley / taffy format to cairo glyphs. Also make sure we
            // offset the glyphs by the base_x and base_y.
            let mut cairo_glyphs = vec![];
            for glyph in &text.glyphs {
                let cairo_glyph = cairo::Glyph::new(
                    glyph.id as u64,
                    base_x + glyph.x as f64 + text.offset.x as f64,
                    base_y + glyph.y as f64 + text.offset.y as f64,
                );
                cairo_glyphs.push(cairo_glyph);
            }

            _ = cr.show_glyphs(&cairo_glyphs);

            // Set decoration (underline, overline, line-through)
            {
                let decoration = &text.decoration;
                let _stroke = Stroke::new(decoration.width as f64);

                let c = decoration.color;
                let brush = GsBrush::solid(GsColor::rgba32(c.0, c.1, c.2, 1.0));
                GsBrush::render(&brush, cr);

                let offset = decoration.x_offset as f64;
                if decoration.underline {
                    let y = base_y + decoration.underline_offset as f64 + obj.rect.height;

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
