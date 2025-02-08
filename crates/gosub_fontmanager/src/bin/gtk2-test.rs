use freetype::Library;
use gosub_fontmanager::{FontInfo, FontManager};
use gosub_interface::font::{FontInfo as _, FontManager as _, FontStyle};
use gtk4::cairo::{FontFace, Glyph};
use gtk4::prelude::*;
use gtk4::{glib, Application, ApplicationWindow, DrawingArea};
use image::Rgba;
use parley::fontique::Weight;
use parley::layout::{Alignment, Layout, PositionedLayoutItem};
use parley::style::StyleProperty;
use parley::{Font, InlineBox, LayoutContext};
use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use rand::Rng;

#[derive(Clone, Copy, Debug, PartialEq)]
struct ColorBrush {
    color: Rgba<u8>,
}

impl Default for ColorBrush {
    fn default() -> Self {
        Self {
            color: Rgba([0, 0, 0, 255]),
        }
    }
}

const APP_ID: &str = "io.gosub.font-manager.gtk-test";

fn main() -> glib::ExitCode {
    colog::init();

    let app = Application::builder().application_id(APP_ID).build();
    app.connect_activate(build_ui);

    app.connect_startup(|_app| {
        println!("Setting default icon");
        gtk4::Window::set_default_icon_name(APP_ID);
    });

    app.run()
}

struct FontContext {
    ft_lib: Rc<Library>,
    font_face_cache: HashMap<u64, FontFace>,
    font_manager: Rc<FontManager>,
    parley_context: parley::FontContext,
}

fn build_ui(app: &Application) {
    // Create a window and set the title
    let window = ApplicationWindow::builder()
        .application(app)
        .title("GTK Font Renderer")
        .build();

    // Setup font context so we initialize these things only once and reuse them for each drawing
    let ft_lib = Rc::new(Library::init().unwrap());
    let font_face_cache = HashMap::new();
    let font_manager = Rc::new(FontManager::new());
    let parley_context = parley::FontContext::new();

    let mut font_context = FontContext {
        ft_lib,
        font_face_cache,
        font_manager,
        parley_context,
    };

    let fonts = ["arial", "verdana", "comic sans ms", "webdings"];
    let current_font_idx = Rc::new(RefCell::new(0));
    let current_font_size = Rc::new(RefCell::new(24.0));

    // let text = "Some text here. Let's make it a bit longer so that line wrapping kicks in 😊. And also some اللغة العربية arabic text.\nThis is underline and strikethrough text";
    let text = gosub_fontmanager::flatland::TEXT;

    let font_size_clone = Rc::clone(&current_font_size);
    let font_idx_clone = Rc::clone(&current_font_idx);

    let area = DrawingArea::default();
    area.set_hexpand(true);
    area.set_vexpand(true);
    area.set_draw_func(move |area, cr, width, _height| {
        let font_info = font_context
            .font_manager
            .find_font(&[fonts[*font_idx_clone.clone().borrow()]], FontStyle::Normal)
            .unwrap();

        // Draw a random colored square to indicate stuff is being (re)drawn on screen
        let mut rng = rand::rng();
        cr.set_source_rgba(rng.random(), rng.random(), rng.random(), 0.5);
        cr.rectangle(0.0, 0.0, 100.0, 100.0);
        let _ = cr.fill();

        let mut offset_y = 100.0;

        let layout = create_layout(
            &mut font_context,
            &font_info,
            text,
            width as f64,
            *font_size_clone.borrow(),
        );
        let layout_height = layout.height();

        draw(&mut font_context, cr, layout, 100.0, offset_y);

        // Add some padding between the different font sizes
        offset_y += layout_height + 25.0;

        // The height is now the total height of all the text. We can se the content height to
        // be the height of this. Since we are drawing this inside a scrollable window, the scroll
        // will kick in when this content height is larger than the window height.
        area.set_content_height(offset_y as i32 + 50);
    });

    let scroll = gtk4::ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Automatic)
        .vscrollbar_policy(gtk4::PolicyType::Automatic)
        .child(&area)
        .build();
    window.set_child(Some(&scroll));

    let controller = gtk4::EventControllerKey::new();

    let font_size_clone = Rc::clone(&current_font_size);
    let font_idx_clone = Rc::new(current_font_idx);

    controller.connect_key_pressed(move |_controller, keyval, _keycode, _state| {
        // Check which key was pressed
        match keyval {
            key if key == gtk4::gdk::Key::a => {
                *font_size_clone.borrow_mut() -= 2.0;
                if *font_size_clone.borrow() < 2.0 {
                    *font_size_clone.borrow_mut() = 2.0;
                }
                println!("Font size: {}", *font_size_clone.borrow());
                area.queue_draw();
            }
            key if key == gtk4::gdk::Key::s => {
                *font_size_clone.borrow_mut() += 2.0;
                println!("Font size: {}", *font_size_clone.borrow());
                area.queue_draw();
            }
            key if key == gtk4::gdk::Key::z => {
                *font_idx_clone.borrow_mut() += 1;
                if *font_idx_clone.borrow() >= fonts.len() {
                    *font_idx_clone.borrow_mut() = 0;
                }
                area.queue_draw();
            }
            key if key == gtk4::gdk::Key::x => {
                if *font_idx_clone.borrow() == 0 {
                    *font_idx_clone.borrow_mut() = fonts.len() - 1;
                } else {
                    *font_idx_clone.borrow_mut() -= 1;
                }
                area.queue_draw();
            }
            _ => (),
        }

        glib::Propagation::Proceed
    });
    window.add_controller(controller);

    // Create a small enough window so we can see scrollbars in the scroll window
    window.set_default_width(800);
    window.set_default_height(600);
    window.present();
}

// Draw the layout onto the cairo context
fn draw(fctx: &mut FontContext, cr: &gtk4::cairo::Context, layout: Layout<ColorBrush>, offset_x: f32, offset_y: f32) {
    // The layouter has cut the text into different lines for us.
    for line in layout.lines() {
        // Each item is either a run of glyps or an inline box.
        for item in line.items() {
            match item {
                PositionedLayoutItem::GlyphRun(glyph_run) => {
                    let grun = glyph_run.run();

                    // Find the font that is accompanied by this glyph run, or generate it if it does not exist yet.
                    let font_id = grun.font().data.id();

                    let font_face = match fctx.font_face_cache.get(&font_id) {
                        Some(font_face) => font_face,
                        None => {
                            let font_face = create_memory_font_face(fctx.ft_lib.clone(), grun.font());
                            fctx.font_face_cache.insert(font_id, font_face);
                            fctx.font_face_cache.get(&font_id).unwrap()
                        }
                    };
                    cr.set_font_face(font_face);

                    cr.set_font_size(glyph_run.run().font_size() as f64);

                    // Render per glyph
                    cr.set_source_rgba(0.0, 0.0, 0.0, 1.0);

                    // Glyphs are already positioned by the layouter. However, we must take into account
                    // that our offset is not 0,0 but offset_x, offset_y.
                    let glyphs: Vec<Glyph> = glyph_run
                        .positioned_glyphs()
                        .map(|g| Glyph::new(g.id as u64, offset_x as f64 + g.x as f64, offset_y as f64 + g.y as f64))
                        .collect();

                    // We can show the set of glyphs as a whole now
                    cr.show_glyphs(glyphs.as_slice()).unwrap();
                }
                PositionedLayoutItem::InlineBox(inline_box) => {
                    cr.rectangle(
                        (offset_x + inline_box.x) as f64,
                        (offset_y + inline_box.y) as f64,
                        inline_box.width as f64,
                        inline_box.height as f64,
                    );
                    cr.set_source_rgba(0.0, 0.0, 0.0, 1.0);
                    let _ = cr.stroke();

                    cr.rectangle(
                        (offset_x + inline_box.x) as f64,
                        (offset_y + inline_box.y) as f64,
                        inline_box.width as f64,
                        inline_box.height as f64,
                    );
                    cr.set_source_rgba(0.0, 0.0, 1.0, 0.25);
                    let _ = cr.fill();
                }
            };
        }
    }
}

/// Creates a cairo font-face from the font data (blob of raw fontdata). We do this by converting
/// the blob into an in-memory freetype face and then into a cairo font face.
fn create_memory_font_face(ft_lib: Rc<Library>, font: &Font) -> FontFace {
    // Create an in-memory font face from the font data
    let face = ft_lib.new_memory_face2(font.data.data(), font.index as isize).unwrap();
    let mut face = face.clone();

    // SAFETY: The user data entry keeps `freetype::face::Face` alive
    // until the FontFace is dropped.
    unsafe {
        FontFace::from_raw_full(cairo::ffi::cairo_ft_font_face_create_for_ft_face(
            face.raw_mut() as cairo::freetype::ffi::FT_Face as *mut _,
            0,
        ))
    }
}

fn create_layout(
    fctx: &mut FontContext,
    font_info: &FontInfo,
    text: &str,
    width: f64,
    font_size: f64,
) -> Layout<ColorBrush> {
    let display_scale = 1.0_f32;

    // Max_advance is the maximum width of a line. We can use this to break lines. We use the width of the window - minus some padding.
    let max_advance = Some((width - 100.0) as f32 * display_scale);

    let mut layout_cx = LayoutContext::new();

    // I'm not 100% clear why the layouter needs a text brush or color?
    let text_color = Rgba([0, 0, 0, 255]);
    let text_brush = ColorBrush { color: text_color };
    let brush_style = StyleProperty::Brush(text_brush);
    let bold_style = StyleProperty::FontWeight(Weight::BOLD);
    // let underline_style = StyleProperty::Underline(true);
    // let strikethrough_style = StyleProperty::Strikethrough(true);

    // Fetch parley from the font manager. Notice that we ask parley for a context and font stack based on the font-family we
    // requested through our font_info.

    let font_stack = parley::FontStack::Single(parley::style::FontFamily::Named(Cow::Borrowed(font_info.family())));

    let mut builder = layout_cx.ranged_builder(&mut fctx.parley_context, text, display_scale);
    builder.push_default(brush_style);
    builder.push_default(font_stack);
    builder.push_default(StyleProperty::LineHeight(1.0));
    builder.push_default(StyleProperty::FontSize(font_size as f32));
    builder.push_default(StyleProperty::LetterSpacing(1.0));

    builder.push(bold_style, 6..11); // From index 6 to 11, the text will be bold
                                     // builder.push(underline_style, 141..150);
                                     // builder.push(strikethrough_style, 155..168);

    // Add some inline boxes. They can represent inline images for instance.
    builder.push_inline_box(InlineBox {
        id: 0,
        index: 5,
        width: 100.0,
        height: 100.0,
    });

    builder.push_inline_box(InlineBox {
        id: 1,
        index: 50,
        width: 100.0,
        height: 30.0,
    });

    let mut layout: Layout<ColorBrush> = builder.build(text);

    // We can now break the lines and align them.
    layout.break_all_lines(max_advance);
    layout.align(max_advance, Alignment::Start);

    layout
}
