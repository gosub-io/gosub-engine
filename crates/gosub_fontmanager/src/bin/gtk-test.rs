use gosub_interface::font::{FontInfo, FontManager, FontStyle};
use gtk4::pango::FontDescription;
use gtk4::prelude::{
    ApplicationExt, ApplicationExtManual, DrawingAreaExt, DrawingAreaExtManual, GtkWindowExt, WidgetExt,
};
use gtk4::{glib, Application, ApplicationWindow, DrawingArea};
use pangocairo::functions::{create_layout, show_layout};
use pangocairo::pango;
use std::cell::RefCell;
use std::rc::Rc;

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
    font_manager: gosub_fontmanager::FontManager,
    // font_map: pango::FontMap,
}

fn build_ui(app: &Application) {
    let font_manager = gosub_fontmanager::FontManager::new();
    // let font_map = pangocairo::FontMap::new();
    let font_context = FontContext {
        font_manager,
        // font_map,
    };

    // See if fonts actually exists
    let _ = font_context
        .font_manager
        .find(&["comic sans ms"], FontStyle::Normal)
        .expect("Failed to find font Comic Sans MS");
    let _ = font_context
        .font_manager
        .find(&["Arial"], FontStyle::Normal)
        .expect("Failed to find font Arial");

    let fonts = ["arial", "verdana", "comic sans ms", "webdings"];
    let current_font_idx = Rc::new(RefCell::new(0));
    let current_font_size = Rc::new(RefCell::new(24.0));

    let font_size_clone = Rc::clone(&current_font_size);
    let font_idx_clone = Rc::clone(&current_font_idx);

    // Create a window and set the title
    let window = ApplicationWindow::builder()
        .application(app)
        .title("GTK Font Renderer")
        .build();

    let area = DrawingArea::default();
    area.set_hexpand(true);
    area.set_vexpand(true);
    area.set_draw_func(move |area, gtk_cr, width, _height| {
        // Red square to indicate stuff is being drawn on screen
        gtk_cr.set_source_rgba(1.0, 0.0, 0.0, 1.0);
        gtk_cr.rectangle(0.0, 0.0, 100.0, 100.0);
        let _ = gtk_cr.fill();

        // Layout works nicely with bounding boxes and alignment, but I can't seem to get the font face to render
        let layout = create_layout(gtk_cr);

        let idx1 = *font_idx_clone.clone().borrow() % fonts.len();
        let idx2 = (*font_idx_clone.clone().borrow() + 1) % fonts.len();
        let fs = *font_size_clone.borrow();

        let fi = font_context
            .font_manager
            .find_font(&[fonts[idx1]], FontStyle::Normal)
            .unwrap();
        let desc = FontDescription::from_string(fi.to_description(fs).as_str());
        layout.set_font_description(Some(&desc));

        layout.set_text(gosub_fontmanager::flatland::TEXT);
        layout.set_width(width * pango::SCALE);
        layout.set_alignment(pango::Alignment::Center);

        let cur_y = 200;
        let mut max_y = cur_y;

        // Create layout
        gtk_cr.set_source_rgba(1.0, 0.0, 1.0, 1.0);
        gtk_cr.move_to(0.0, f64::from(cur_y));
        show_layout(gtk_cr, &layout);
        max_y += layout.pixel_size().1;

        // Nice bounding rectangle around the text
        gtk_cr.set_source_rgba(0.0, 0.0, 0.0, 1.0);
        gtk_cr.set_line_width(1.0);
        gtk_cr.rectangle(
            0.0,
            f64::from(cur_y),
            f64::from(width),
            f64::from(max_y) - f64::from(cur_y),
        );
        let _ = gtk_cr.stroke();

        // Add a little bit of padding
        max_y += 25;
        let cur_y = max_y;

        // Display the next text in a different font
        let fi = font_context
            .font_manager
            .find_font(&[fonts[idx2]], FontStyle::Normal)
            .unwrap();
        let desc = FontDescription::from_string(fi.to_description(fs).as_str());
        layout.set_font_description(Some(&desc));
        gtk_cr.set_source_rgba(0.7, 0.2, 0.5, 1.0);
        gtk_cr.move_to(0.0, f64::from(cur_y));
        show_layout(gtk_cr, &layout);
        max_y += layout.pixel_size().1;

        // Bounding box around the text again
        gtk_cr.set_source_rgba(0.0, 1.0, 1.0, 1.0);
        gtk_cr.set_line_width(3.0);
        gtk_cr.rectangle(
            0.0,
            f64::from(cur_y),
            f64::from(width),
            f64::from(max_y) - f64::from(cur_y),
        );
        let _ = gtk_cr.stroke();

        // Get current position and add the layout height. This is the new height of the canvas in this drawing area so
        // we can scroll.
        area.set_content_height(max_y + 50);
    });

    // Of course, scrolling doesn't work... need to figure out why it doesn't work.
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

    window.set_default_width(800);
    window.set_default_height(600);
    window.present();
}
