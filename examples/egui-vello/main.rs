//! Minimal browser window: Vello (GPU) rasterizer + egui toolkit.
//!
//! Usage: cargo run --example egui-vello -- https://example.com
//!
//! No GTK dependency — pure egui + wgpu.

use eframe::{egui, CreationContext};
use gosub_engine::events::{EngineEvent, NavigationEvent, TabCommand};
use gosub_engine::storage::{InMemorySessionStore, PartitionPolicy, SqliteLocalStore, StorageService};
use gosub_engine::tab::{TabDefaults, TabHandle, TabId};
use gosub_engine::zone::{Zone, ZoneConfig, ZoneId, ZoneServices};
use gosub_engine::GosubEngine;
use gosub_render_pipeline::render::backend::{blend_over_argb_u32, ExternalHandle};
use gosub_render_pipeline::render::{DefaultCompositor, Viewport};
use gosub_renderer_vello::{VelloBackend, WgpuContextProvider};
use once_cell::sync::Lazy;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::runtime::{Builder, Runtime};
use url::Url;
use uuid::uuid;
use vello::wgpu;

const DEFAULT_ZONE: uuid::Uuid = uuid!("f1234567-abcd-4000-8000-000000000008");

static TOKIO_RT: Lazy<Runtime> = Lazy::new(|| {
    Builder::new_multi_thread()
        .enable_io()
        .enable_time()
        .thread_name("gosub-egui-vello-rt")
        .build()
        .expect("tokio runtime")
});

// ── wgpu context provider (backed by eframe's wgpu state) ───────────────────

struct EguiContextProvider {
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    textures: RwLock<HashMap<u64, (wgpu::Texture, wgpu::TextureView)>>,
    next_id: AtomicU64,
}

impl EguiContextProvider {
    fn from_eframe(cc: &CreationContext) -> Option<Self> {
        let ws = cc.wgpu_render_state.as_ref()?;
        Some(Self {
            device: Arc::new(ws.device.clone()),
            queue: Arc::new(ws.queue.clone()),
            textures: RwLock::new(HashMap::new()),
            next_id: AtomicU64::new(1),
        })
    }
}

impl WgpuContextProvider for EguiContextProvider {
    fn device(&self) -> &wgpu::Device {
        &self.device
    }

    fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }

    fn device_arc(&self) -> Arc<wgpu::Device> {
        Arc::clone(&self.device)
    }

    fn queue_arc(&self) -> Arc<wgpu::Queue> {
        Arc::clone(&self.queue)
    }

    fn create_texture(&self, width: u32, height: u32, format: wgpu::TextureFormat) -> u64 {
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("gosub-vello-texture"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.textures.write().insert(id, (texture, view));
        id
    }

    fn get_texture(&self, id: u64) -> Option<(wgpu::Texture, wgpu::TextureView)> {
        self.textures
            .read()
            .get(&id)
            .map(|(t, v): &(wgpu::Texture, wgpu::TextureView)| (t.clone(), v.clone()))
    }

    fn remove_texture(&self, id: u64) {
        self.textures.write().remove(&id);
    }
}

// ── UI events ────────────────────────────────────────────────────────────────

enum UiEvent {
    LocationChanged { url: String },
    NavigationStarted,
    NavigationFinished,
    HoverUrl(Option<String>),
}

// ── Application ──────────────────────────────────────────────────────────────

struct BrowserApp {
    #[allow(dead_code)]
    engine: GosubEngine,
    #[allow(dead_code)]
    zone: Zone,
    tab: TabHandle,
    tab_id: TabId,
    compositor: Arc<RwLock<DefaultCompositor>>,
    context: Arc<EguiContextProvider>,

    url_input: String,
    status_url: String,
    /// CPU texture for TileCache frames (pipeline+vello path).
    cpu_texture: Option<egui::TextureHandle>,
    /// GPU texture id for WgpuTextureId frames (non-pipeline vello path).
    egui_texture: Option<(u64, egui::TextureId)>,
    last_panel_size: egui::Vec2,
    ui_rx: std::sync::mpsc::Receiver<UiEvent>,
    is_loading: bool,
    scroll_x: f32,
    scroll_y: f32,
    page_height: f32,
}

impl BrowserApp {
    fn new(cc: &CreationContext, initial_url: String) -> Option<Self> {
        let _rt = TOKIO_RT.enter();

        let ctx = cc.egui_ctx.clone();
        let compositor = Arc::new(RwLock::new(DefaultCompositor::new(move || {
            ctx.request_repaint();
        })));

        let context = Arc::new(EguiContextProvider::from_eframe(cc)?);
        let backend = VelloBackend::new(context.clone()).ok()?;

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
                    // Vello needs a non-zero viewport to create the wgpu texture.
                    // The real size is sent via SetViewport on the first panel resize.
                    viewport: Some(Viewport::new(0, 0, 1024, 768)),
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

        Some(Self {
            engine,
            zone,
            tab,
            tab_id,
            compositor,
            context,
            url_input: initial_url,
            status_url: String::new(),
            cpu_texture: None,
            egui_texture: None,
            last_panel_size: egui::Vec2::ZERO,
            ui_rx,
            is_loading: true,
            scroll_x: 0.0,
            scroll_y: 0.0,
            page_height: 0.0,
        })
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

    /// Register or refresh the display texture from the latest engine frame.
    /// Handles both TileCache (pipeline+vello CPU path) and WgpuTextureId (raw vello GPU path).
    fn refresh_texture(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        let Some(handle) = self.compositor.read().frame_for(self.tab_id) else {
            return;
        };

        match handle {
            ExternalHandle::TileCache {
                tiles,
                dpr,
                viewport_width,
                viewport_height,
                page_height,
                ..
            } => {
                self.page_height = page_height;

                let dpr_f = dpr as f32;
                let w = (viewport_width * dpr) as usize;
                let h = (viewport_height * dpr) as usize;
                if w == 0 || h == 0 {
                    return;
                }
                let sx = (self.scroll_x * dpr_f) as i64;
                let sy = (self.scroll_y * dpr_f) as i64;
                // Opaque white: a valid premultiplied background for source-over blending.
                let mut buf = vec![0xFFFF_FFFFu32; w * h];

                for tile in tiles.iter() {
                    let px = (tile.page_x * dpr_f) as i64;
                    let py = (tile.page_y * dpr_f) as i64;
                    let screen_x = px - sx;
                    let screen_y = py - sy;
                    let tw = tile.width as i64;
                    let th = tile.height as i64;
                    if screen_x >= w as i64 || screen_y >= h as i64 {
                        continue;
                    }
                    if screen_x + tw <= 0 || screen_y + th <= 0 {
                        continue;
                    }
                    let tile_start_col = (-screen_x).max(0) as usize;
                    let tile_start_row = (-screen_y).max(0) as usize;
                    let dst_x = screen_x.max(0) as usize;
                    let dst_y0 = screen_y.max(0) as usize;
                    let tw = tw as usize;
                    let th = th as usize;
                    // Normalize to [R, G, B, A] regardless of which rasterizer produced the tile
                    // (Cargo feature unification may select Cairo's ARGB32 over Vello's RGBA).
                    let tile_data = tile.format.to_rgba(&tile.data);
                    let tile_u32 =
                        unsafe { std::slice::from_raw_parts(tile_data.as_ptr() as *const u32, tile_data.len() / 4) };
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
                        // content beneath, instead of overwriting it. `buf` and `tile_u32`
                        // are both [R,G,B,A]; the blend is channel-symmetric, so the
                        // swapped R/B (vs. ARGB) does not affect the result.
                        for col in 0..copy_w {
                            buf[dst_off + col] = blend_over_argb_u32(tile_u32[src_off + col], buf[dst_off + col]);
                        }
                    }
                }

                // `buf` now holds [R, G, B, A] bytes (normalized per-tile above).
                let mut rgba = Vec::with_capacity(w * h * 4);
                for &px in &buf {
                    let r = (px & 0xFF) as u8;
                    let g = ((px >> 8) & 0xFF) as u8;
                    let b = ((px >> 16) & 0xFF) as u8;
                    rgba.extend_from_slice(&[r, g, b, 255]);
                }

                let img = egui::ColorImage::from_rgba_unmultiplied([w, h], &rgba);
                match &mut self.cpu_texture {
                    Some(t) => t.set(img, egui::TextureOptions::LINEAR),
                    None => self.cpu_texture = Some(ctx.load_texture("browser", img, egui::TextureOptions::LINEAR)),
                }
            }

            ExternalHandle::WgpuTextureId { id, frame_id, .. } => {
                let needs_update = self
                    .egui_texture
                    .as_ref()
                    .map(|(fid, _)| *fid != frame_id)
                    .unwrap_or(true);
                if !needs_update {
                    return;
                }
                let Some(wgpu_state) = frame.wgpu_render_state() else {
                    return;
                };
                let Some((_, view)) = self.context.get_texture(id) else {
                    return;
                };
                if let Some((_, old)) = self.egui_texture.take() {
                    wgpu_state.renderer.write().free_texture(&old);
                }
                let new_tex = wgpu_state.renderer.write().register_native_texture(
                    &self.context.device,
                    &view,
                    eframe::wgpu::FilterMode::Linear,
                );
                self.egui_texture = Some((frame_id, new_tex));
            }

            _ => {}
        }
    }
}

impl eframe::App for BrowserApp {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

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

        self.refresh_texture(&ctx, frame);

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

            // Prefer CPU tile-cache texture (pipeline+vello path), fall back to GPU texture.
            let tex_id = self
                .cpu_texture
                .as_ref()
                .map(|t| t.id())
                .or_else(|| self.egui_texture.as_ref().map(|(_, id)| *id));

            if let Some(tex_id) = tex_id {
                let (rect, response) = ui.allocate_exact_size(panel_size, egui::Sense::click());

                ui.painter().image(
                    tex_id,
                    rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );

                // Mouse move → hover
                if let Some(pos) = ctx.pointer_latest_pos() {
                    if rect.contains(pos) {
                        let rel = pos - rect.min;
                        let tab = self.tab.clone();
                        TOKIO_RT.spawn(async move {
                            let _ = tab.send(TabCommand::MouseMove { x: rel.x, y: rel.y }).await;
                        });
                    }
                }

                // Click → links
                if response.clicked() {
                    if let Some(pos) = ctx.input(|i| i.pointer.interact_pos()) {
                        let rel = pos - rect.min;
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
            .with_title("Gosub Browser — egui + Vello")
            .with_inner_size([1024.0, 768.0]),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };

    eframe::run_native(
        "Gosub Browser",
        options,
        Box::new(move |cc| {
            cc.egui_ctx.set_visuals(egui::Visuals::light());
            BrowserApp::new(cc, initial_url)
                .map(|app| Box::new(app) as Box<dyn eframe::App>)
                .ok_or_else(|| "wgpu render state not available — eframe must use the wgpu renderer".into())
        }),
    )
}
