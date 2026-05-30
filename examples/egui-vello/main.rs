use crate::tiling::{
    close_leaf, collect_leaves, compute_layout, find_leaf_at, split_leaf_into_cols, split_leaf_into_rows, LayoutHandle,
    LayoutNode, Rect,
};
use crate::wgpu_context_provider::EguiWgpuContextProvider;
use eframe::{egui, CreationContext};
use egui::StrokeKind;
use gosub_engine::cookies::SqliteCookieStore;
use gosub_engine::events::{EngineEvent, NavigationEvent, TabCommand};
use gosub_engine::storage::{InMemorySessionStore, PartitionPolicy, SqliteLocalStore, StorageService};
use gosub_engine::tab::{TabDefaults, TabHandle, TabId};
use gosub_engine::zone::{Zone, ZoneConfig, ZoneId, ZoneServices};
use gosub_engine::GosubEngine;
use gosub_render_pipeline::render::backend::ExternalHandle;
use gosub_render_pipeline::render::backends::vello::VelloBackend;
use gosub_render_pipeline::render::{DefaultCompositor, Viewport};
use gosub_shared::tab_id::TabId as SharedTabId;
use once_cell::sync::Lazy;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::runtime::{Builder, Runtime};
use url::Url;
use uuid::uuid;

mod tiling;
mod wgpu_context_provider;

const DEFAULT_MAIN_ZONE: uuid::Uuid = uuid!("95d9c701-5f1b-43ea-ba7e-bc509ee8aa54");
const DEFAULT_WIDTH: u32 = 800;
const DEFAULT_HEIGHT: u32 = 600;

static TOKIO_RT: Lazy<Runtime> = Lazy::new(|| {
    Builder::new_multi_thread()
        .enable_io()
        .enable_time()
        .thread_name("gosub-rt")
        .build()
        .expect("init tokio runtime")
});

/// Events forwarded from the engine event bus to the UI thread.
enum UiEvent {
    LocationChanged { tab_id: TabId, url: String },
    TitleChanged { _tab_id: TabId, _title: String },
    NavigationStarted { tab_id: TabId },
    NavigationFinished { tab_id: TabId },
}

struct GosubApp {
    #[allow(dead_code)]
    engine: GosubEngine,
    zone: Zone,
    root: LayoutHandle,
    active_tab: TabId,
    tabs: HashMap<TabId, TabHandle>,
    last_size: (i32, i32),
    current_url_input: String,
    /// Current URL per tab (updated from LocationChanged engine events).
    tab_urls: HashMap<TabId, String>,
    pointer_pos: (f64, f64),
    last_panel_size: egui::Vec2,
    /// Latest rendered frame per tab, populated by the compositor callback.
    compositor_frames: Arc<RwLock<HashMap<SharedTabId, ExternalHandle>>>,
    /// wgpu context provider shared with VelloBackend.
    wgpu_provider: Arc<EguiWgpuContextProvider>,
    /// Per-tab cached egui TextureId: (frame_id, egui_texture_id).
    tab_textures: HashMap<TabId, (u64, egui::TextureId)>,
    /// Engine events relevant to the UI (URL bar, title, loading state).
    ui_rx: std::sync::mpsc::Receiver<UiEvent>,
    /// True while the active tab is still loading.
    is_loading: bool,
}

impl GosubApp {
    fn new(cc: &CreationContext) -> Self {
        let _rt_guard = TOKIO_RT.enter();

        let provider = Arc::new(
            EguiWgpuContextProvider::from_eframe(cc)
                .expect("egui-vello requires a wgpu render context; ensure eframe uses the wgpu backend"),
        );

        let backend: Arc<dyn gosub_render_pipeline::render::RenderBackend + Send + Sync> =
            Arc::new(VelloBackend::new(Arc::clone(&provider)).expect("VelloBackend::new failed"));

        // Wire the compositor's redraw callback directly to egui's repaint request.
        // This means egui repaints only when the engine actually produces a new frame.
        let ctx_for_compositor = cc.egui_ctx.clone();
        let compositor = Arc::new(RwLock::new(DefaultCompositor::new(move || {
            ctx_for_compositor.request_repaint();
        })));
        let compositor_frames = compositor.read().frames_arc();

        let mut engine = GosubEngine::new(None, backend, compositor);
        let _engine_join_handle = engine.start().expect("Engine start failed");

        // Forward relevant engine events to the UI thread via a std channel so ui() can
        // drain them synchronously without holding a lock or using async.
        let (ui_tx, ui_rx) = std::sync::mpsc::channel::<UiEvent>();
        let mut event_rx = engine.subscribe_events();
        let ctx_for_events = cc.egui_ctx.clone();

        TOKIO_RT.spawn(async move {
            loop {
                match event_rx.recv().await {
                    Ok(ev) => {
                        let ui_ev = match ev {
                            EngineEvent::LocationChanged { tab_id, url } => {
                                Some(UiEvent::LocationChanged { tab_id, url })
                            }
                            EngineEvent::TitleChanged { tab_id, title }
                            | EngineEvent::TabTitleChanged { tab_id, title } => Some(UiEvent::TitleChanged {
                                _tab_id: tab_id,
                                _title: title,
                            }),
                            EngineEvent::Navigation {
                                tab_id,
                                event: NavigationEvent::Started { .. },
                            } => Some(UiEvent::NavigationStarted { tab_id }),
                            EngineEvent::Navigation {
                                tab_id,
                                event: NavigationEvent::Finished { .. } | NavigationEvent::Failed { .. },
                            } => Some(UiEvent::NavigationFinished { tab_id }),
                            _ => None,
                        };

                        if let Some(ev) = ui_ev {
                            let _ = ui_tx.send(ev);
                            ctx_for_events.request_repaint();
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        log::warn!("UI event receiver lagged by {} events", n);
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });

        let zone_cfg = ZoneConfig::builder()
            .do_not_track(true)
            .accept_languages("fr-CH, fr;q=0.9, en;q=0.8, de;q=0.7, *;q=0.5")
            .build()
            .expect("ZoneConfig is not valid");

        let sqlite_store = SqliteCookieStore::new(".gosub-gtk-cookie-store.db".into());
        let cookie_store: gosub_engine::cookies::CookieStoreHandle = sqlite_store.into();

        let zone_services = ZoneServices {
            storage: Arc::new(StorageService::new(
                Arc::new(SqliteLocalStore::new(".gosub-gtk-local-storage.db").unwrap()),
                Arc::new(InMemorySessionStore::new()),
            )),
            cookie_store: Some(cookie_store),
            cookie_jar: None,
            partition_policy: PartitionPolicy::None,
        };

        let mut zone = engine
            .create_zone(zone_cfg, zone_services, Some(ZoneId::from(DEFAULT_MAIN_ZONE)))
            .expect("create_zone failed");

        let mut tabs: HashMap<TabId, TabHandle> = HashMap::new();

        let tab = TOKIO_RT
            .block_on(async {
                let tab = zone
                    .create_tab(
                        TabDefaults {
                            url: None,
                            title: Some("New Tab".to_string()),
                            viewport: Some(Viewport::new(0, 0, DEFAULT_WIDTH, DEFAULT_HEIGHT)),
                        },
                        None,
                    )
                    .await?;
                let _ = tab.send(TabCommand::ResumeDrawing { fps: 60 }).await;
                Ok::<TabHandle, gosub_engine::EngineError>(tab)
            })
            .expect("create_tab failed");

        let tab_id = tab.tab_id;
        tabs.insert(tab_id, tab);

        if let Some(t) = tabs.get(&tab_id).cloned() {
            TOKIO_RT.spawn(async move {
                let _ = t.navigate("https://example.com").await;
            });
        }

        let active_tab = tab_id;
        let root: LayoutHandle = std::rc::Rc::new(std::cell::RefCell::new(LayoutNode::Leaf(active_tab)));

        Self {
            engine,
            zone,
            root,
            active_tab,
            tabs,
            last_size: (DEFAULT_WIDTH as i32, DEFAULT_HEIGHT as i32),
            current_url_input: String::new(),
            tab_urls: HashMap::new(),
            pointer_pos: (0.0, 0.0),
            last_panel_size: egui::Vec2::ZERO,
            compositor_frames,
            wgpu_provider: provider,
            tab_textures: HashMap::new(),
            ui_rx,
            is_loading: true,
        }
    }

    fn handle_navigation(&mut self, ctx: egui::Context) {
        let composed_url = self.current_url_input.clone();

        let url_str = if !composed_url.starts_with("http://") && !composed_url.starts_with("https://") {
            format!("https://{}", composed_url)
        } else {
            composed_url
        };

        let Ok(url) = Url::parse(&url_str) else {
            return;
        };

        self.is_loading = true;

        let tab_id = self.active_tab;
        if let Some(tab) = self.tabs.get(&tab_id).cloned() {
            TOKIO_RT.spawn(async move {
                let _ = tab.navigate(url.as_str()).await;
                let _ = tab.send(TabCommand::ResumeDrawing { fps: 60 }).await;
                ctx.request_repaint();
            });
        }
    }

    fn handle_split_col(&mut self) {
        let (w, h) = self.last_size;
        let new_tab = TOKIO_RT
            .block_on(async {
                let tab = self
                    .zone
                    .create_tab(
                        TabDefaults {
                            url: None,
                            title: Some("New Tab".to_string()),
                            viewport: Some(Viewport::new(0, 0, (w / 2).max(1) as u32, h as u32)),
                        },
                        None,
                    )
                    .await?;
                let _ = tab.send(TabCommand::ResumeDrawing { fps: 60 }).await;
                Ok::<TabHandle, gosub_engine::EngineError>(tab)
            })
            .expect("create_tab failed");

        let new_id = new_tab.tab_id;
        self.tabs.insert(new_id, new_tab);

        let target = self.active_tab;
        split_leaf_into_cols(&self.root, target, vec![new_id]);

        let mut pairs = Vec::new();
        compute_layout(&self.root.borrow(), Rect { x: 0, y: 0, w, h }, &mut pairs);
        for (tab_id, r) in pairs {
            if let Some(tab) = self.tabs.get(&tab_id).cloned() {
                TOKIO_RT.spawn(async move {
                    let _ = tab.set_viewport(Viewport::new(0, 0, r.w as u32, r.h as u32)).await;
                });
            }
        }
    }

    fn handle_split_row(&mut self) {
        let (w, h) = self.last_size;
        let new_tab = TOKIO_RT
            .block_on(async {
                let tab = self
                    .zone
                    .create_tab(
                        TabDefaults {
                            url: None,
                            title: Some("New Tab".to_string()),
                            viewport: Some(Viewport::new(0, 0, w as u32, (h / 2).max(1) as u32)),
                        },
                        None,
                    )
                    .await?;
                let _ = tab.send(TabCommand::ResumeDrawing { fps: 60 }).await;
                Ok::<TabHandle, gosub_engine::EngineError>(tab)
            })
            .expect("create_tab failed");

        let new_id = new_tab.tab_id;
        self.tabs.insert(new_id, new_tab);

        let target = self.active_tab;
        split_leaf_into_rows(&self.root, target, vec![new_id]);

        let mut pairs = Vec::new();
        compute_layout(&self.root.borrow(), Rect { x: 0, y: 0, w, h }, &mut pairs);
        for (tab_id, r) in pairs {
            if let Some(tab) = self.tabs.get(&tab_id).cloned() {
                TOKIO_RT.spawn(async move {
                    let _ = tab.set_viewport(Viewport::new(0, 0, r.w as u32, r.h as u32)).await;
                });
            }
        }
    }

    fn handle_close_pane(&mut self) {
        let target = self.active_tab;
        if close_leaf(&self.root, target) {
            let mut leaves = Vec::new();
            collect_leaves(&self.root.borrow(), &mut leaves);
            if let Some(&first) = leaves.first() {
                self.active_tab = first;
            }
            let (w, h) = self.last_size;
            let mut pairs = Vec::new();
            compute_layout(&self.root.borrow(), Rect { x: 0, y: 0, w, h }, &mut pairs);
            for (tab_id, r) in pairs {
                if let Some(tab) = self.tabs.get(&tab_id).cloned() {
                    TOKIO_RT.spawn(async move {
                        let _ = tab.set_viewport(Viewport::new(0, 0, r.w as u32, r.h as u32)).await;
                    });
                }
            }
        }
    }

    fn handle_click(&mut self, pos: egui::Pos2) {
        let (w, h) = self.last_size;
        if let Some(tab_id) = find_leaf_at(
            &self.root.borrow(),
            Rect { x: 0, y: 0, w, h },
            pos.x as f64,
            pos.y as f64,
        ) {
            self.active_tab = tab_id;
        }
    }

    fn handle_scroll(&mut self, delta: egui::Vec2) {
        let (px, py) = self.pointer_pos;
        let (w, h) = self.last_size;

        if let Some(tab_id) = find_leaf_at(&self.root.borrow(), Rect { x: 0, y: 0, w, h }, px, py) {
            let line_h = 2.0;
            let dx_px = delta.x * line_h;
            let dy_px = delta.y * line_h;

            if let Some(tab) = self.tabs.get(&tab_id).cloned() {
                TOKIO_RT.spawn(async move {
                    let _ = tab
                        .send(TabCommand::MouseScroll {
                            delta_x: dx_px,
                            delta_y: dy_px,
                        })
                        .await;
                });
            }
        }
    }
}

impl eframe::App for GosubApp {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        ctx.global_style_mut(|style| {
            style.text_styles.get_mut(&egui::TextStyle::Body).unwrap().size = 16.0;
            style.text_styles.get_mut(&egui::TextStyle::Button).unwrap().size = 14.0;
        });

        if let Some(pointer_pos) = ctx.pointer_latest_pos() {
            self.pointer_pos = (pointer_pos.x as f64, pointer_pos.y as f64);
        }

        let scroll_delta = ctx.input(|i| i.smooth_scroll_delta);
        if scroll_delta != egui::Vec2::ZERO {
            self.handle_scroll(scroll_delta);
        }

        if ctx.input(|i| i.pointer.primary_clicked()) {
            if let Some(click_pos) = ctx.input(|i| i.pointer.interact_pos()) {
                self.handle_click(click_pos);
            }
        }

        // Drain engine events: URL bar updates, title changes, load state.
        while let Ok(ev) = self.ui_rx.try_recv() {
            match ev {
                UiEvent::LocationChanged { tab_id, url } => {
                    self.tab_urls.insert(tab_id, url.clone());
                    if tab_id == self.active_tab {
                        self.current_url_input = url;
                    }
                }
                UiEvent::TitleChanged { .. } => {}
                UiEvent::NavigationStarted { tab_id } => {
                    if tab_id == self.active_tab {
                        self.is_loading = true;
                    }
                }
                UiEvent::NavigationFinished { tab_id } => {
                    if tab_id == self.active_tab {
                        self.is_loading = false;
                    }
                }
            }
        }

        egui::Panel::top("address_bar")
            .frame(egui::Frame::default().inner_margin(egui::Margin::symmetric(12, 10)))
            .show_inside(ui, |ui| {
                ui.horizontal(|ui| {
                    if self.is_loading {
                        ui.spinner();
                    }

                    let response = ui.add(
                        egui::TextEdit::singleline(&mut self.current_url_input)
                            .desired_width(f32::INFINITY)
                            .hint_text("Enter URL")
                            .char_limit(100)
                            .font(egui::TextStyle::Heading)
                            .min_size(egui::vec2(0.0, 36.0)),
                    );

                    if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        self.handle_navigation(ctx.clone());
                    }
                });
            });

        egui::Panel::top("toolbar")
            .frame(egui::Frame::default().inner_margin(egui::Margin::symmetric(12, 8)))
            .show_inside(ui, |ui| {
                ui.horizontal(|ui| {
                    let button_size = egui::vec2(120.0, 32.0);

                    if ui.add(egui::Button::new("Split Col").min_size(button_size)).clicked() {
                        self.handle_split_col();
                    }

                    if ui.add(egui::Button::new("Split Row").min_size(button_size)).clicked() {
                        self.handle_split_row();
                    }

                    if ui.add(egui::Button::new("Close Pane").min_size(button_size)).clicked() {
                        self.handle_close_pane();
                    }
                });
            });

        egui::CentralPanel::default().show_inside(ui, |ui| {
            // Capture available_size before adding any widgets. This is the stable measurement
            // for both layout and viewport resize: it is unaffected by whatever we render into
            // the panel, preventing the feedback loop where content overflow inflates the size
            // reported by panel.response.rect, which in turn triggers a resize, which inflates
            // the content again.
            let available_size = ui.available_size();

            if available_size != self.last_panel_size {
                self.last_panel_size = available_size;

                let mut pairs = Vec::new();
                compute_layout(
                    &self.root.borrow(),
                    Rect {
                        x: 0,
                        y: 0,
                        w: available_size.x as i32,
                        h: available_size.y as i32,
                    },
                    &mut pairs,
                );

                for (tab_id, r) in pairs {
                    if let Some(tab) = self.tabs.get(&tab_id).cloned() {
                        TOKIO_RT.spawn(async move {
                            let _ = tab.set_viewport(Viewport::new(0, 0, r.w as u32, r.h as u32)).await;
                        });
                    }
                }
            }

            self.last_size = (available_size.x as i32, available_size.y as i32);

            let (w, h) = self.last_size;
            let mut pairs = Vec::new();
            compute_layout(&self.root.borrow(), Rect { x: 0, y: 0, w, h }, &mut pairs);

            let active_tab_id = self.active_tab;

            let frames_snapshot: HashMap<TabId, ExternalHandle> = {
                let guard = self.compositor_frames.read();
                guard.iter().map(|(k, v)| (*k, v.clone())).collect()
            };

            for (tab_id, rect) in pairs {
                let is_active = tab_id == active_tab_id;

                let rect_ui = egui::Rect::from_min_max(
                    egui::pos2(rect.x as f32, rect.y as f32),
                    egui::pos2((rect.x + rect.w) as f32, (rect.y + rect.h) as f32),
                );

                ui.painter().rect_filled(rect_ui, 0.0, egui::Color32::WHITE);

                let mut rendered = false;
                if let Some(ExternalHandle::WgpuTextureId { id, frame_id, .. }) = frames_snapshot.get(&tab_id).cloned()
                {
                    if let Some(wgpu_state) = frame.wgpu_render_state() {
                        let needs_register = self
                            .tab_textures
                            .get(&tab_id)
                            .map(|(fid, _)| *fid != frame_id)
                            .unwrap_or(true);

                        if needs_register {
                            if let Some((_, old_tex)) = self.tab_textures.remove(&tab_id) {
                                wgpu_state.renderer.write().free_texture(&old_tex);
                            }
                            if let Some((_, view)) = self.wgpu_provider.get_texture(id) {
                                let new_tex = wgpu_state.renderer.write().register_native_texture(
                                    &self.wgpu_provider.device,
                                    &view,
                                    eframe::wgpu::FilterMode::Nearest,
                                );
                                self.tab_textures.insert(tab_id, (frame_id, new_tex));
                            }
                        }

                        if let Some((_, egui_tex)) = self.tab_textures.get(&tab_id) {
                            // Draw directly to the painter at rect_ui rather than adding a
                            // widget. A widget allocates layout space based on the texture's
                            // pixel dimensions, which can exceed available_size and start the
                            // feedback loop we fixed above.
                            ui.painter().image(
                                *egui_tex,
                                rect_ui,
                                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                                egui::Color32::WHITE,
                            );
                            rendered = true;
                        }
                    }
                }

                if !rendered {
                    ui.scope_builder(egui::UiBuilder::new().max_rect(rect_ui), |ui| {
                        ui.centered_and_justified(|ui| {
                            ui.label(
                                egui::RichText::new("No frame yet — navigate to a URL")
                                    .italics()
                                    .color(egui::Color32::GRAY),
                            );
                        });
                    });
                }

                let col = if is_active {
                    egui::Color32::YELLOW
                } else {
                    egui::Color32::DARK_GRAY
                };
                ui.painter()
                    .rect_stroke(rect_ui, 0.0, egui::Stroke::new(2.0, col), StrokeKind::Outside);
            }
        });
    }
}

fn main() -> Result<(), eframe::Error> {
    simple_logger::SimpleLogger::new()
        .with_level(log::LevelFilter::Info)
        .init()
        .ok();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_title("Gosub Browser - Egui + Vello"),
        ..Default::default()
    };

    eframe::run_native(
        "Gosub Browser",
        options,
        Box::new(|cc| {
            cc.egui_ctx.set_visuals(egui::Visuals::light());
            Ok(Box::new(GosubApp::new(cc)))
        }),
    )
}
