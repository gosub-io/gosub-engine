//! Minimal browser window: Cairo rasterizer + egui toolkit.
//!
//! Usage: cargo run --example egui-cairo -- https://example.com
//!
//! Cairo/Pango need GTK4 initialised for font rendering (no GTK window is created).
//! On headless systems set GDK_BACKEND=offscreen.

use eframe::{egui, CreationContext};
use gosub_engine::events::{EngineEvent, NavigationEvent, TabCommand};
use gosub_engine::storage::{InMemorySessionStore, PartitionPolicy, SqliteLocalStore, StorageService};
use gosub_engine::tab::{TabDefaults, TabHandle, TabId};
use gosub_engine::zone::{Zone, ZoneConfig, ZoneId, ZoneServices};
use gosub_engine::GosubEngine;
use gosub_render_pipeline::render::backend::{blend_over_argb_u32, ExternalHandle};
use gosub_renderer_cairo::{CairoBackend, DEVICE_PIXEL_RATIO};
use gosub_render_pipeline::render::DefaultCompositor;
use once_cell::sync::Lazy;
use parking_lot::RwLock;
use std::sync::Arc;
use tokio::runtime::{Builder, Runtime};
use url::Url;
use uuid::uuid;

const DEFAULT_ZONE: uuid::Uuid = uuid!("f1234567-abcd-4000-8000-000000000005");

static TOKIO_RT: Lazy<Runtime> = Lazy::new(|| {
    Builder::new_multi_thread()
        .enable_io()
        .enable_time()
        .thread_name("gosub-egui-cairo-rt")
        .build()
        .expect("tokio runtime")
});

enum UiEvent {
    LocationChanged { url: String },
    NavigationStarted,
    NavigationFinished,
    HoverUrl(Option<String>),
}

struct BrowserApp {
    #[allow(dead_code)]
    engine: GosubEngine,
    #[allow(dead_code)]
    zone: Zone,
    tab: TabHandle,
    tab_id: TabId,
    compositor: Arc<RwLock<DefaultCompositor>>,
    url_input: String,
    status_url: String,
    texture: Option<egui::TextureHandle>,
    last_panel_size: egui::Vec2,
    ui_rx: std::sync::mpsc::Receiver<UiEvent>,
    is_loading: bool,
    scroll_x: f32,
    scroll_y: f32,
    page_height: f32,
}

impl BrowserApp {
    fn new(cc: &CreationContext, initial_url: String) -> Self {
        let _rt = TOKIO_RT.enter();

        let ctx = cc.egui_ctx.clone();
        let compositor = Arc::new(RwLock::new(DefaultCompositor::new(move || {
            ctx.request_repaint();
        })));

        let backend = CairoBackend::new();
        let mut engine = GosubEngine::new(None, Arc::new(backend), compositor.clone());
        let _join = engine.start().expect("engine start");

        let (ui_tx, ui_rx) = std::sync::mpsc::channel::<UiEvent>();
        let mut event_rx = engine.subscribe_events();
        let ctx_ev = cc.egui_ctx.clone();
        TOKIO_RT.spawn(async move {
            loop {
                match event_rx.recv().await {
                    Ok(ev) => {
                        let out = match ev {
                            EngineEvent::LocationChanged { url, .. } => Some(UiEvent::LocationChanged { url }),
                            EngineEvent::Navigation {
                                event: NavigationEvent::Started { .. },
                                ..
                            } => Some(UiEvent::NavigationStarted),
                            EngineEvent::Navigation {
                                event: NavigationEvent::Finished { .. } | NavigationEvent::Failed { .. },
                                ..
                            } => Some(UiEvent::NavigationFinished),
                            EngineEvent::HoverUrl { url, .. } => Some(UiEvent::HoverUrl(url)),
                            _ => None,
                        };
                        if let Some(ev) = out {
                            let _ = ui_tx.send(ev);
                            ctx_ev.request_repaint();
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });

        let zone_cfg = ZoneConfig::builder().do_not_track(true).build().expect("ZoneConfig");
        let zone_services = ZoneServices {
            storage: Arc::new(StorageService::new(
                Arc::new(SqliteLocalStore::new(":memory:").expect("local store")),
                Arc::new(InMemorySessionStore::new()),
            )),
            cookie_store: None,
            cookie_jar: None,
            partition_policy: PartitionPolicy::None,
        };

        let mut zone = engine
            .create_zone(zone_cfg, zone_services, Some(ZoneId::from(DEFAULT_ZONE)))
            .expect("create_zone");

        let tab = TOKIO_RT
            .block_on(zone.create_tab(
                TabDefaults {
                    url: None,
                    title: Some("Gosub".to_string()),
                    viewport: None,
                },
                None,
            ))
            .expect("create_tab");

        let tab_id = tab.tab_id;
        let nav_tab = tab.clone();
        let nav_url = initial_url.clone();
        TOKIO_RT.spawn(async move {
            let _ = nav_tab.send(TabCommand::Navigate { url: nav_url }).await;
            let _ = nav_tab.send(TabCommand::ResumeDrawing { fps: 30 }).await;
        });

        Self {
            engine,
            zone,
            tab,
            tab_id,
            compositor,
            url_input: initial_url,
            status_url: String::new(),
            texture: None,
            last_panel_size: egui::Vec2::ZERO,
            ui_rx,
            is_loading: true,
            scroll_x: 0.0,
            scroll_y: 0.0,
            page_height: 0.0,
        }
    }

    fn navigate(&mut self) {
        let mut s = self.url_input.clone();
        if !s.starts_with("http://") && !s.starts_with("https://") {
            s = format!("https://{s}");
            self.url_input = s.clone();
        }
        let Ok(_) = Url::parse(&s) else { return };
        self.is_loading = true;
        self.scroll_x = 0.0;
        self.scroll_y = 0.0;
        let tab = self.tab.clone();
        TOKIO_RT.spawn(async move {
            let _ = tab.send(TabCommand::Navigate { url: s }).await;
            let _ = tab.send(TabCommand::ResumeDrawing { fps: 30 }).await;
        });
    }

    fn refresh_texture(&mut self, ctx: &egui::Context) {
        let Some(handle) = self.compositor.read().frame_for(self.tab_id) else {
            return;
        };

        let (w, h, rgba) = match handle {
            ExternalHandle::CpuPixelsOwned {
                width,
                height,
                stride,
                pixels,
                ..
            } => {
                let rgba = argb32_to_rgba8(&pixels, width as usize, height as usize, stride as usize);
                (width as usize, height as usize, rgba)
            }
            ExternalHandle::CpuPixelsPtr {
                width,
                height,
                stride,
                pixel_buf,
            } => {
                let bytes =
                    unsafe { std::slice::from_raw_parts(pixel_buf.as_ptr(), height as usize * stride as usize) };
                let rgba = argb32_to_rgba8(bytes, width as usize, height as usize, stride as usize);
                (width as usize, height as usize, rgba)
            }
            ExternalHandle::TileCache {
                tiles,
                dpr,
                viewport_width,
                viewport_height,
                page_height,
                ..
            } => {
                // Update page_height so scroll clamping stays accurate.
                self.page_height = page_height;

                let dpr_f = dpr as f32;
                let w = (viewport_width * dpr) as usize;
                let h = (viewport_height * dpr) as usize;
                if w == 0 || h == 0 {
                    return;
                }
                // Physical-pixel scroll offset using local state (no async roundtrip).
                let sx = (self.scroll_x * dpr_f) as i64;
                let sy = (self.scroll_y * dpr_f) as i64;
                // Opaque white: a valid premultiplied background for source-over blending.
                let mut buf = vec![0xFFFF_FFFFu32; w * h];

                for tile in tiles.iter() {
                    // Physical-pixel position of this tile on the page.
                    let px = (tile.page_x * dpr_f) as i64;
                    let py = (tile.page_y * dpr_f) as i64;
                    // Screen position — may be negative when tile starts above/left of viewport.
                    let screen_x = px - sx;
                    let screen_y = py - sy;
                    let tw = tile.width as i64;
                    let th = tile.height as i64;
                    // Cull tiles fully outside the viewport.
                    if screen_x >= w as i64 || screen_y >= h as i64 {
                        continue;
                    }
                    if screen_x + tw <= 0 || screen_y + th <= 0 {
                        continue;
                    }
                    // When a tile starts before the viewport edge, skip the off-screen rows/cols.
                    let tile_start_col = (-screen_x).max(0) as usize;
                    let tile_start_row = (-screen_y).max(0) as usize;
                    let dst_x = screen_x.max(0) as usize;
                    let dst_y0 = screen_y.max(0) as usize;
                    let tw = tw as usize;
                    let th = th as usize;
                    let tile_u32 =
                        unsafe { std::slice::from_raw_parts(tile.data.as_ptr() as *const u32, tile.data.len() / 4) };
                    for tile_row in tile_start_row..th {
                        let dst_y = dst_y0 + (tile_row - tile_start_row);
                        if dst_y >= h {
                            break;
                        }
                        let copy_w = (tw - tile_start_col).min(w - dst_x);
                        if copy_w == 0 {
                            break;
                        }
                        let src_off = tile_row * tw + tile_start_col;
                        let dst_off = dst_y * w + dst_x;
                        // Source-over blend so transparent upper-layer pixels reveal the
                        // content beneath, instead of overwriting it.
                        for col in 0..copy_w {
                            let src_argb = tile.format.pixel_to_argb_u32(tile_u32[src_off + col]);
                            buf[dst_off + col] = blend_over_argb_u32(src_argb, buf[dst_off + col]);
                        }
                    }
                }

                let mut rgba = Vec::with_capacity(w * h * 4);
                for &px in &buf {
                    let b = (px & 0xFF) as u8;
                    let g = ((px >> 8) & 0xFF) as u8;
                    let r = ((px >> 16) & 0xFF) as u8;
                    rgba.extend_from_slice(&[r, g, b, 255]);
                }
                (w, h, rgba)
            }
            _ => return,
        };

        if w == 0 || h == 0 {
            return;
        }

        let img = egui::ColorImage::from_rgba_unmultiplied([w, h], &rgba);
        match &mut self.texture {
            Some(t) => t.set(img, egui::TextureOptions::LINEAR),
            None => {
                self.texture = Some(ctx.load_texture("browser", img, egui::TextureOptions::LINEAR));
            }
        }
    }
}

/// Convert Cairo ARGB32 (premultiplied, LE bytes: B G R A) to egui RGBA8 (straight alpha).
fn argb32_to_rgba8(pixels: &[u8], width: usize, height: usize, stride: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(width * height * 4);
    for row in 0..height {
        for col in 0..width {
            let off = row * stride + col * 4;
            let b = pixels[off];
            let g = pixels[off + 1];
            let r = pixels[off + 2];
            let a = pixels[off + 3];
            if a == 0 {
                out.extend_from_slice(&[0, 0, 0, 0]);
            } else if a == 255 {
                out.extend_from_slice(&[r, g, b, 255]);
            } else {
                let af = a as f32 / 255.0;
                out.push((r as f32 / af).min(255.0) as u8);
                out.push((g as f32 / af).min(255.0) as u8);
                out.push((b as f32 / af).min(255.0) as u8);
                out.push(a);
            }
        }
    }
    out
}

impl eframe::App for BrowserApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        // Keep Cairo's DPR in sync with egui's pixel ratio.
        let dpr = ctx.pixels_per_point().round() as u32;
        DEVICE_PIXEL_RATIO.store(dpr.max(1), std::sync::atomic::Ordering::Relaxed);

        // Drain engine events.
        while let Ok(ev) = self.ui_rx.try_recv() {
            match ev {
                UiEvent::LocationChanged { url } => self.url_input = url,
                UiEvent::NavigationStarted => self.is_loading = true,
                UiEvent::NavigationFinished => self.is_loading = false,
                UiEvent::HoverUrl(url) => self.status_url = url.unwrap_or_default(),
            }
        }

        // Update local scroll synchronously so refresh_texture composites at the new position.
        let scroll_delta = ctx.input(|i| i.smooth_scroll_delta);
        if scroll_delta != egui::Vec2::ZERO {
            let dx = -scroll_delta.x;
            let dy = -scroll_delta.y;
            let max_y = (self.page_height - self.last_panel_size.y).max(0.0);
            self.scroll_x = (self.scroll_x + dx).max(0.0);
            self.scroll_y = (self.scroll_y + dy).clamp(0.0, max_y);
            let tab = self.tab.clone();
            TOKIO_RT.spawn(async move {
                let _ = tab
                    .send(TabCommand::MouseScroll {
                        delta_x: dx,
                        delta_y: dy,
                    })
                    .await;
            });
        }

        self.refresh_texture(&ctx);

        // Address bar
        egui::Panel::top("addr")
            .frame(egui::Frame::default().inner_margin(egui::Margin::symmetric(8, 6)))
            .show_inside(ui, |ui| {
                ui.horizontal(|ui| {
                    if self.is_loading {
                        ui.spinner();
                    }
                    let r = ui.add(
                        egui::TextEdit::singleline(&mut self.url_input)
                            .desired_width(f32::INFINITY)
                            .hint_text("Enter URL…")
                            .font(egui::TextStyle::Monospace),
                    );
                    if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        self.navigate();
                    }
                });
            });

        // Status bar
        egui::Panel::bottom("status")
            .frame(egui::Frame::default().inner_margin(egui::Margin::symmetric(4, 2)))
            .show_inside(ui, |ui| {
                ui.label(egui::RichText::new(&self.status_url).small());
            });

        // Browser content
        egui::CentralPanel::default().show_inside(ui, |ui| {
            let panel_size = ui.available_size();

            if panel_size != self.last_panel_size && panel_size.x > 0.0 && panel_size.y > 0.0 {
                self.last_panel_size = panel_size;
                let tab = self.tab.clone();
                let w = panel_size.x as u32;
                let h = panel_size.y as u32;
                TOKIO_RT.spawn(async move {
                    let _ = tab
                        .send(TabCommand::SetViewport {
                            x: 0,
                            y: 0,
                            width: w,
                            height: h,
                        })
                        .await;
                });
            }

            if let Some(texture) = &self.texture {
                let response = ui.add(egui::Image::new(texture).fit_to_exact_size(panel_size));

                // Mouse move → hover
                if let Some(pos) = ctx.pointer_latest_pos() {
                    if response.rect.contains(pos) {
                        let rel = pos - response.rect.min;
                        let tab = self.tab.clone();
                        TOKIO_RT.spawn(async move {
                            let _ = tab.send(TabCommand::MouseMove { x: rel.x, y: rel.y }).await;
                        });
                    }
                }

                // Click → links
                if response.clicked() {
                    if let Some(pos) = ctx.input(|i| i.pointer.interact_pos()) {
                        let rel = pos - response.rect.min;
                        let tab = self.tab.clone();
                        TOKIO_RT.spawn(async move {
                            let _ = tab
                                .send(TabCommand::MouseDown {
                                    x: rel.x,
                                    y: rel.y,
                                    button: gosub_engine::events::MouseButton::Left,
                                })
                                .await;
                        });
                    }
                }
            } else {
                ui.centered_and_justified(|ui| {
                    ui.label(egui::RichText::new("Loading…").italics().color(egui::Color32::GRAY));
                });
            }
        });
    }
}

fn main() -> Result<(), eframe::Error> {
    simple_logger::SimpleLogger::new()
        .with_level(log::LevelFilter::Warn)
        .env()
        .init()
        .unwrap_or_default();

    // Cairo/Pango need GTK4 initialised for font rendering. No GTK window is created.
    gosub_engine::init_gtk_resources().expect("failed to init GTK resources");

    let initial_url = {
        let raw = std::env::args()
            .nth(1)
            .unwrap_or_else(|| "https://example.com".to_string());
        if raw.contains("://") {
            raw
        } else {
            format!("https://{raw}")
        }
    };

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Gosub Browser — egui + Cairo")
            .with_inner_size([1024.0, 768.0]),
        ..Default::default()
    };

    eframe::run_native(
        "Gosub Browser",
        options,
        Box::new(move |cc| {
            cc.egui_ctx.set_visuals(egui::Visuals::light());
            Ok(Box::new(BrowserApp::new(cc, initial_url)))
        }),
    )
}
