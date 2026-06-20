//! Browser window: Skia GPU (OpenGL/Ganesh) rasterizer + GTK4 GLArea.
//!
//! Usage: cargo run --example gtk4-skia-gpu -- https://example.com
//!
//! This is the only fully GPU-native example — no CPU readback:
//!   Engine layout (CPU) → Skia GPU rasterization → GTK4 GLArea framebuffer → display
//!
//! Architecture:
//!   1. `SkiaGpuDirectFbBackend` captures the display list on the engine thread.
//!   2. `GLArea::connect_render` fires on the GTK main thread with GL current.
//!   3. Skia renders the display list directly into GTK4's framebuffer.

// Link libGL so glGetIntegerv resolves (used to query GTK4's bound FBO).
#[link(name = "GL")]
extern "C" {}

use gosub_engine::cookies::SqliteCookieStore;
use gosub_engine::events::{EngineEvent, NavigationEvent, TabCommand};
use gosub_engine::storage::{InMemorySessionStore, PartitionPolicy, SqliteLocalStore, StorageService};
use gosub_engine::tab::{TabDefaults, TabId};
use gosub_engine::zone::{ZoneConfig, ZoneId, ZoneServices};
use gosub_engine::GosubEngine;
use gosub_render_pipeline::render::backends::skia_gpu::{to_color4f, PendingFrame, SkiaGpuDirectFbBackend};
use gosub_render_pipeline::render::render_list::DisplayItem;
use gosub_render_pipeline::render::DefaultCompositor;
use gtk4::glib;
use gtk4::prelude::*;
use gtk4::{Application, ApplicationWindow, Box as GtkBox, Entry, GLArea, Label, Orientation};
use once_cell::sync::Lazy;
use parking_lot::RwLock;
use skia_safe::{Font, FontMgr, FontStyle, Paint, Rect as SkRect};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;
use tokio::runtime::{Builder, Runtime};
use tokio::sync::mpsc;
use url::Url;
use uuid::uuid;

const DEFAULT_ZONE: uuid::Uuid = uuid!("f1234567-abcd-4000-8000-00000000000d");
const SCROLL_MULTIPLIER: f32 = 12.5;

static TOKIO_RT: Lazy<Runtime> = Lazy::new(|| {
    Builder::new_multi_thread()
        .enable_io()
        .enable_time()
        .thread_name("gosub-gtk4-skia-gpu-rt")
        .build()
        .expect("tokio runtime")
});

// ── GL helper ─────────────────────────────────────────────────────────────────

/// Query the currently bound draw framebuffer ID from OpenGL.
fn get_bound_fbo() -> u32 {
    extern "C" {
        fn glGetIntegerv(pname: u32, data: *mut i32);
    }
    let mut fbo = 0i32;
    unsafe {
        glGetIntegerv(0x8CA6 /* GL_DRAW_FRAMEBUFFER_BINDING */, &mut fbo)
    };
    fbo as u32
}

// ── Skia GPU rendering ────────────────────────────────────────────────────────

fn render_frame_to_gl(dc: &mut skia_safe::gpu::DirectContext, frame: &PendingFrame, width: i32, height: i32) {
    if width == 0 || height == 0 {
        return;
    }

    let fbo = get_bound_fbo();
    let fb_info = skia_safe::gpu::gl::FramebufferInfo {
        fboid: fbo,
        format: 0x8058, // GL_RGBA8
        protected: skia_safe::gpu::Protected::No,
    };
    let target = skia_safe::gpu::backend_render_targets::make_gl((width, height), None, 8, fb_info);

    let Some(mut surface) = skia_safe::gpu::surfaces::wrap_backend_render_target(
        dc,
        &target,
        skia_safe::gpu::SurfaceOrigin::BottomLeft, // OpenGL: y=0 at bottom
        skia_safe::ColorType::RGBA8888,
        None,
        None,
    ) else {
        log::warn!("gtk4-skia-gpu: failed to wrap framebuffer as Skia surface");
        return;
    };

    {
        let canvas = surface.canvas();
        canvas.clip_rect(SkRect::new(0.0, 0.0, width as f32, height as f32), None, None);
        canvas.save();
        canvas.translate((-frame.offset_x, -frame.offset_y));

        for item in &frame.items {
            render_item(canvas, item);
        }

        canvas.restore();
    }

    // Flush to GPU without waiting for CPU — GTK4 synchronises before compositing.
    dc.flush_surface(&mut surface);
    dc.submit(skia_safe::gpu::SyncCpu::No);
}

fn render_item(canvas: &skia_safe::Canvas, item: &DisplayItem) {
    match item {
        DisplayItem::Clear { color } => {
            canvas.clear(to_color4f(color));
        }
        DisplayItem::Rect { x, y, w, h, color } => {
            let mut paint = Paint::new(to_color4f(color), None);
            paint.set_anti_alias(true);
            canvas.draw_rect(SkRect::new(*x, *y, x + w, y + h), &paint);
        }
        DisplayItem::TextRun {
            x,
            y,
            text,
            size,
            color,
            ..
        } => {
            thread_local! { static FONT_MGR: FontMgr = FontMgr::new(); }
            let typeface = FONT_MGR.with(|fm| {
                fm.legacy_make_typeface(None, FontStyle::normal()).unwrap_or_else(|| {
                    fm.legacy_make_typeface("sans-serif", FontStyle::normal())
                        .expect("no typeface")
                })
            });
            let font = Font::new(typeface, *size);
            let mut paint = Paint::new(to_color4f(color), None);
            paint.set_anti_alias(true);
            canvas.draw_str(text.as_str(), (*x, *y), &font, &paint);
        }
        DisplayItem::Blit { x, y, w, h, data } => {
            let stride = (*w * 4) as usize;
            if data.len() < *h as usize * stride {
                return;
            }
            let info = skia_safe::ImageInfo::new(
                (*w as i32, *h as i32),
                skia_safe::ColorType::BGRA8888,
                skia_safe::AlphaType::Premul,
                None,
            );
            if let Some(image) = skia_safe::images::raster_from_data(&info, skia_safe::Data::new_copy(data), stride) {
                canvas.draw_image(&image, (*x, *y), None);
            }
        }
    }
}

// ── Application ───────────────────────────────────────────────────────────────

fn main() {
    simple_logger::SimpleLogger::new()
        .with_level(log::LevelFilter::Warn)
        .env()
        .init()
        .unwrap_or_default();

    let initial_url: String = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "https://example.com".to_string());

    let app = Application::builder().application_id("io.gosub.gtk4-skia-gpu").build();

    app.connect_activate(move |app| {
        let _rt_guard = TOKIO_RT.enter();

        gosub_engine::init_gtk_resources();

        let (tx_redraw, mut rx_redraw) = mpsc::unbounded_channel::<()>();

        let compositor = Arc::new(RwLock::new(DefaultCompositor::new({
            let tx = tx_redraw.clone();
            move || {
                let _ = tx.send(());
            }
        })));

        let backend = SkiaGpuDirectFbBackend::new();
        let pending = backend.pending.clone();

        let mut engine = GosubEngine::new(None, Arc::new(backend), compositor.clone());
        let _join = engine.start().expect("engine start");
        let event_rx = engine.subscribe_events();

        let zone_cfg = ZoneConfig::builder().do_not_track(true).build().expect("ZoneConfig");
        let cookie_store: gosub_engine::cookies::CookieStoreHandle =
            SqliteCookieStore::new(".pipeline-browser-cookies.db".into()).into();
        let zone_services = ZoneServices {
            storage: Arc::new(StorageService::new(
                Arc::new(SqliteLocalStore::new(".pipeline-browser-local.db").expect("local store")),
                Arc::new(InMemorySessionStore::new()),
            )),
            cookie_store: Some(cookie_store),
            cookie_jar: None,
            partition_policy: PartitionPolicy::None,
        };

        let zone = Rc::new(RefCell::new(
            engine
                .create_zone(zone_cfg, zone_services, Some(ZoneId::from(DEFAULT_ZONE)))
                .expect("create_zone"),
        ));

        let tab = TOKIO_RT
            .block_on(zone.borrow_mut().create_tab(
                TabDefaults {
                    url: None,
                    title: Some("Gosub".to_string()),
                    viewport: None,
                },
                None,
            ))
            .expect("create_tab");

        let _tab_id: TabId = tab.tab_id;
        let tab = Rc::new(RefCell::new(tab));
        let local_scroll: Rc<Cell<(f32, f32)>> = Rc::new(Cell::new((0.0, 0.0)));

        // ── GLArea ────────────────────────────────────────────────────────────

        let gl_area = GLArea::new();
        gl_area.set_has_depth_buffer(false);
        gl_area.set_has_stencil_buffer(true); // Skia needs stencil
        gl_area.set_vexpand(true);
        gl_area.set_hexpand(true);
        gl_area.set_focusable(true);

        // Hold Skia's DirectContext: created in connect_realize, used in connect_render.
        let dc_holder: Rc<RefCell<Option<skia_safe::gpu::DirectContext>>> = Rc::new(RefCell::new(None));

        // Initialise Skia's GL DirectContext once the GLArea is realized.
        gl_area.connect_realize({
            let dc_holder = dc_holder.clone();
            move |area| {
                area.make_current();
                if area.error().is_some() {
                    log::error!("GLArea realize error: {:?}", area.error());
                    return;
                }
                let interface = skia_safe::gpu::gl::Interface::new_native()
                    .expect("Skia GL interface — GLArea context must be current");
                let dc = skia_safe::gpu::direct_contexts::make_gl(interface, None).expect("Skia DirectContext");
                *dc_holder.borrow_mut() = Some(dc);
                log::info!("gtk4-skia-gpu: Skia DirectContext initialised");
            }
        });

        // Render pending display list directly into GTK4's framebuffer.
        gl_area.connect_render({
            let dc_holder = dc_holder.clone();
            let pending = pending.clone();
            move |area, _ctx| {
                let mut dc_ref = dc_holder.borrow_mut();
                let Some(dc) = dc_ref.as_mut() else {
                    return glib::Propagation::Stop;
                };
                let Some(frame) = pending.lock().take() else {
                    return glib::Propagation::Stop;
                };
                render_frame_to_gl(dc, &frame, area.width(), area.height());
                glib::Propagation::Stop
            }
        });

        // Trigger a GLArea redraw whenever the engine has a new frame.
        {
            let gl_area = gl_area.clone();
            glib::spawn_future_local(async move {
                while let Some(()) = rx_redraw.recv().await {
                    gl_area.queue_render();
                }
            });
        }

        // ── Resize ───────────────────────────────────────────────────────────

        gl_area.connect_resize({
            let tab = tab.clone();
            let local_scroll = local_scroll.clone();
            move |_, w, h| {
                local_scroll.set((0.0, 0.0));
                let tab = tab.borrow().clone();
                TOKIO_RT.spawn(async move {
                    let _ = tab
                        .send(TabCommand::SetViewport {
                            x: 0,
                            y: 0,
                            width: w as u32,
                            height: h as u32,
                        })
                        .await;
                });
            }
        });

        // ── Scroll ───────────────────────────────────────────────────────────

        let scroll_ctl = gtk4::EventControllerScroll::new(
            gtk4::EventControllerScrollFlags::BOTH_AXES | gtk4::EventControllerScrollFlags::KINETIC,
        );
        scroll_ctl.connect_scroll({
            let tab = tab.clone();
            let local_scroll = local_scroll.clone();
            move |_, dx, dy| {
                let dx = dx as f32 * SCROLL_MULTIPLIER;
                let dy = dy as f32 * SCROLL_MULTIPLIER;
                let (px, py) = local_scroll.get();
                local_scroll.set(((px + dx).max(0.0), (py + dy).max(0.0)));
                let tab = tab.borrow().clone();
                TOKIO_RT.spawn(async move {
                    let _ = tab
                        .send(TabCommand::MouseScroll {
                            delta_x: dx,
                            delta_y: dy,
                        })
                        .await;
                });
                glib::Propagation::Stop
            }
        });
        gl_area.add_controller(scroll_ctl);

        // ── Mouse ────────────────────────────────────────────────────────────

        let motion_ctl = gtk4::EventControllerMotion::new();
        motion_ctl.connect_motion({
            let tab = tab.clone();
            move |_, x, y| {
                let tab = tab.borrow().clone();
                TOKIO_RT.spawn(async move {
                    let _ = tab
                        .send(TabCommand::MouseMove {
                            x: x as f32,
                            y: y as f32,
                        })
                        .await;
                });
            }
        });
        gl_area.add_controller(motion_ctl);

        let click_ctl = gtk4::GestureClick::new();
        click_ctl.set_button(gtk4::gdk::BUTTON_PRIMARY);
        click_ctl.connect_pressed({
            let tab = tab.clone();
            move |_, _, x, y| {
                let tab = tab.borrow().clone();
                TOKIO_RT.spawn(async move {
                    let _ = tab
                        .send(TabCommand::MouseDown {
                            x: x as f32,
                            y: y as f32,
                            button: gosub_engine::events::MouseButton::Left,
                        })
                        .await;
                });
            }
        });
        gl_area.add_controller(click_ctl);

        // ── Address bar ──────────────────────────────────────────────────────

        let address_entry = Entry::new();
        address_entry.set_placeholder_text(Some("Enter URL…"));
        address_entry.set_hexpand(true);

        address_entry.connect_activate({
            let tab = tab.clone();
            let local_scroll = local_scroll.clone();
            move |entry| {
                let mut s = entry.text().to_string();
                if !s.starts_with("http://") && !s.starts_with("https://") {
                    s = format!("https://{s}");
                    entry.set_text(&s);
                }
                let Ok(url) = Url::parse(&s) else { return };
                local_scroll.set((0.0, 0.0));
                let tab = tab.borrow().clone();
                TOKIO_RT.spawn(async move {
                    let _ = tab.send(TabCommand::Navigate { url: url.to_string() }).await;
                    let _ = tab.send(TabCommand::ResumeDrawing { fps: 30 }).await;
                });
            }
        });

        // ── Status bar ───────────────────────────────────────────────────────

        let status_label = Label::new(None);
        status_label.set_halign(gtk4::Align::Start);
        status_label.set_margin_start(4);
        status_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);

        // ── Engine events → UI ───────────────────────────────────────────────

        let (ui_tx, mut ui_rx) = mpsc::unbounded_channel::<EngineEvent>();
        TOKIO_RT.spawn({
            let ui_tx = ui_tx.clone();
            let mut rx = event_rx;
            async move {
                while let Ok(evt) = rx.recv().await {
                    let _ = ui_tx.send(evt);
                }
            }
        });

        glib::spawn_future_local({
            let address_entry = address_entry.clone();
            let status_label = status_label.clone();
            async move {
                while let Some(evt) = ui_rx.recv().await {
                    match evt {
                        EngineEvent::Navigation {
                            event: NavigationEvent::Finished { url, .. },
                            ..
                        } => {
                            address_entry.set_text(url.as_str());
                        }
                        EngineEvent::HoverUrl { url, .. } => {
                            status_label.set_text(url.as_deref().unwrap_or(""));
                        }
                        _ => {}
                    }
                }
            }
        });

        // ── Layout ───────────────────────────────────────────────────────────

        let url_bar = GtkBox::new(Orientation::Horizontal, 0);
        url_bar.append(&address_entry);

        let vbox = GtkBox::new(Orientation::Vertical, 0);
        vbox.append(&url_bar);
        vbox.append(&gl_area);
        vbox.append(&status_label);

        let window = ApplicationWindow::builder()
            .application(app)
            .title("Gosub Browser — GTK4 + Skia GPU")
            .default_width(1024)
            .default_height(800)
            .child(&vbox)
            .build();

        window.present();

        // Navigate after the window is shown.
        {
            let tab_init = tab.clone();
            let url_str = initial_url.clone();
            glib::idle_add_local_once(move || {
                let mut s = url_str.clone();
                if !s.starts_with("http://") && !s.starts_with("https://") {
                    s = format!("https://{s}");
                }
                if let Ok(url) = Url::parse(&s) {
                    let tab = tab_init.borrow().clone();
                    TOKIO_RT.spawn(async move {
                        let _ = tab.send(TabCommand::Navigate { url: url.to_string() }).await;
                        let _ = tab.send(TabCommand::ResumeDrawing { fps: 30 }).await;
                    });
                }
            });
        }
    });

    let argv0: Vec<String> = std::env::args().take(1).collect();
    app.run_with_args(&argv0);
}
