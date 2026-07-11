//! Minimal browser window: Skia (CPU) rasterizer + egui toolkit.
//!
//! Usage: cargo run --example egui-skia -- https://example.com
//!
//! No GTK dependency — Skia has its own font system.

use eframe::{egui, CreationContext};
use gosub_engine::events::{EngineEvent, NavigationEvent, TabCommand};
use gosub_engine::storage::{InMemorySessionStore, PartitionPolicy, SqliteLocalStore, StorageService};
use gosub_engine::tab::{TabDefaults, TabHandle, TabId};
use gosub_engine::zone::{Zone, ZoneConfig, ZoneId, ZoneServices};
use gosub_engine::DefaultRenderConfig;
use gosub_engine::GosubEngine;
use gosub_render_pipeline::render::backend::ExternalHandle;
use gosub_render_pipeline::render::{argb_u32_to_rgba8, composite_tiles, DefaultCompositor, TileTarget, DEVICE_PIXEL_RATIO};
use gosub_renderer_skia::{SkiaBackend, SkiaFontSystem};
use once_cell::sync::Lazy;
use std::sync::Arc;
use tokio::runtime::{Builder, Runtime};
use url::Url;
use uuid::uuid;

const DEFAULT_ZONE: uuid::Uuid = uuid!("f1234567-abcd-4000-8000-000000000009");

type AppConfig = DefaultRenderConfig<SkiaBackend, SkiaFontSystem>;

static TOKIO_RT: Lazy<Runtime> = Lazy::new(|| {
    Builder::new_multi_thread()
        .enable_io()
        .enable_time()
        .thread_name("gosub-egui-skia-rt")
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
    engine: GosubEngine<AppConfig>,
    #[allow(dead_code)]
    zone: Zone<AppConfig>,
    tab: TabHandle,
    tab_id: TabId,
    compositor: Arc<DefaultCompositor>,
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
        let compositor = Arc::new(DefaultCompositor::new(move || {
            ctx.request_repaint();
        }));

        let backend = SkiaBackend::new();
        let mut engine = GosubEngine::<AppConfig>::new(None, Arc::new(backend), compositor.clone());
        let _engine_task = TOKIO_RT.spawn(engine.start().expect("engine start"));

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
        let Some(handle) = self.compositor.frame_for(self.tab_id) else {
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
                let rgba = bgra_premul_to_rgba8(&pixels, width as usize, height as usize, stride as usize);
                (width as usize, height as usize, rgba)
            }
            ExternalHandle::TileCache {
                tiles,
                dpr,
                viewport_width,
                viewport_height,
                page_height,
                scroll_x,
                scroll_y,
            } => {
                self.page_height = page_height;

                let w = (viewport_width * dpr) as usize;
                let h = (viewport_height * dpr) as usize;
                if w == 0 || h == 0 {
                    return;
                }
                // Opaque white background, composite the visible tiles, convert to RGBA8 for egui.
                let mut buf = vec![0xFFFF_FFFFu32; w * h];
                composite_tiles(
                    &tiles,
                    dpr,
                    (scroll_x, scroll_y),
                    &mut TileTarget {
                        buf: &mut buf,
                        stride: w,
                        origin_x: 0,
                        origin_y: 0,
                        width: w,
                        height: h,
                    },
                );
                (w, h, argb_u32_to_rgba8(&buf))
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

/// Convert Skia N32 / Cairo ARGB32 (BGRA premul, LE) → egui RGBA8 straight alpha.
fn bgra_premul_to_rgba8(pixels: &[u8], width: usize, height: usize, stride: usize) -> Vec<u8> {
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

        // Publish the DPR so the Skia backend rasterizes tiles at physical resolution. Use ceil so
        // fractional scaling (e.g. 1.25) renders at 2x and egui downscales — crisp — rather than 1x
        // upscaled (blurry). Matches gtk4-skia / egui-cairo.
        DEVICE_PIXEL_RATIO.store(
            (ctx.pixels_per_point().ceil() as u32).max(1),
            std::sync::atomic::Ordering::Relaxed,
        );

        while let Ok(ev) = self.ui_rx.try_recv() {
            match ev {
                UiEvent::LocationChanged { url } => self.url_input = url,
                UiEvent::NavigationStarted => self.is_loading = true,
                UiEvent::NavigationFinished => self.is_loading = false,
                UiEvent::HoverUrl(url) => self.status_url = url.unwrap_or_default(),
            }
        }

        // Update local scroll synchronously so refresh_texture composites at the new position.
        // Raw mouse-wheel deltas (NOT egui's smoothed scroll): the engine owns scroll smoothing
        // now, so forwarding egui's `smooth_scroll_delta` double-smooths (slow ramp, no ease-out,
        // drawn out). Read raw wheel events and convert lines → px (~134/notch, like the others).
        let scroll_delta = ctx.input(|i| {
            let mut acc = egui::Vec2::ZERO;
            for e in &i.events {
                if let egui::Event::MouseWheel { unit, delta, .. } = e {
                    let mult = match unit {
                        egui::MouseWheelUnit::Line => 134.0,
                        // macOS reports a mouse-wheel notch as an integer Point delta (±1), while a
                        // trackpad reports precise fractional deltas. Treat whole-number notches like
                        // Line events (×134) so the wheel matches the other backends; pass precise
                        // (fractional) trackpad deltas through unscaled.
                        egui::MouseWheelUnit::Point => {
                            if delta.x.fract() == 0.0 && delta.y.fract() == 0.0 {
                                134.0
                            } else {
                                1.0
                            }
                        }
                        egui::MouseWheelUnit::Page => 800.0,
                    };
                    acc += *delta * mult;
                }
            }
            acc
        });
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

        egui::Panel::bottom("status")
            .frame(egui::Frame::default().inner_margin(egui::Margin::symmetric(4, 2)))
            .show_inside(ui, |ui| {
                ui.label(egui::RichText::new(&self.status_url).small());
            });

        egui::CentralPanel::default().show_inside(ui, |ui| {
            let panel_size = ui.available_size();

            if panel_size != self.last_panel_size && panel_size.x > 0.0 && panel_size.y > 0.0 {
                self.last_panel_size = panel_size;
                let tab = self.tab.clone();
                let (w, h) = (panel_size.x as u32, panel_size.y as u32);
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

                if let Some(pos) = ctx.pointer_latest_pos() {
                    if response.rect.contains(pos) {
                        let rel = pos - response.rect.min;
                        let tab = self.tab.clone();
                        TOKIO_RT.spawn(async move {
                            let _ = tab.send(TabCommand::MouseMove { x: rel.x, y: rel.y }).await;
                        });
                    }
                }

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
            .with_title("Gosub Browser — egui + Skia")
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
