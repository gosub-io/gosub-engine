use crate::tiling::{
    close_leaf, collect_leaves, compute_layout, find_leaf_at, split_leaf_into_cols, split_leaf_into_rows, LayoutHandle,
    LayoutNode, Rect,
};
use crate::wgpu_context_provider::EguiWgpuContextProvider;
use eframe::{egui, CreationContext};
use egui::load::SizedTexture;
use egui::StrokeKind;
use gosub_engine_api::cookies::SqliteCookieStore;
use gosub_engine_api::events::{EngineEvent, NavigationEvent, TabCommand};
use gosub_engine_api::render::backend::ExternalHandle;
use gosub_engine_api::render::{DefaultCompositor, Viewport};
use gosub_engine_api::storage::{InMemorySessionStore, PartitionPolicy, SqliteLocalStore, StorageService};
use gosub_engine_api::tab::{TabDefaults, TabHandle, TabId};
use gosub_engine_api::zone::{Zone, ZoneConfig, ZoneId, ZoneServices};
use gosub_engine_api::GosubEngine;
use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use tokio::runtime::{Builder, Runtime};
use url::Url;
use uuid::uuid;

mod compositor;
mod tiling;
mod wgpu_context_provider;

/// We use a fixed UUID for the main zone in this example. This allows us to easily
/// reconnect to the same zone if we restart the app and want to keep cookies/localStorage.
const DEFAULT_MAIN_ZONE: uuid::Uuid = uuid!("95d9c701-5f1b-43ea-ba7e-bc509ee8aa54");
const DEFAULT_WIDTH: u32 = 800;
const DEFAULT_HEIGHT: u32 = 600;

// Global Tokio runtime. This is needed to run the Gosub engine
static TOKIO_RT: Lazy<Runtime> = Lazy::new(|| {
    Builder::new_multi_thread()
        .enable_io()
        .enable_time()
        .thread_name("gosub-rt")
        .build()
        .expect("init tokio runtime")
});

struct GosubApp {
    engine: GosubEngine,
    zone: Zone,
    root: LayoutHandle,
    active_tab: TabId,
    tabs: HashMap<TabId, TabHandle>,
    last_size: (i32, i32),
    compositor: Arc<RwLock<DefaultCompositor>>,
    current_url_input: String,
    needs_redraw: bool,
    pointer_pos: (f64, f64),
    backend_initialized: bool,
    ctx_provider: Arc<EguiWgpuContextProvider>,
    last_panel_size: egui::Vec2,
}

impl GosubApp {
    fn new(cc: &CreationContext) -> Self {
        // Enter Tokio so any internal tokio::spawn/time/io in engine works
        let _rt_guard = TOKIO_RT.enter();

        let compositor = Arc::new(RwLock::new(DefaultCompositor::default()));

        let backend = gosub_engine_api::render::backends::null::NullBackend::new().expect("NullBackend::new failed");
        let mut engine = GosubEngine::new(None, Arc::new(backend), compositor.clone());
        let _engine_join_handle = engine.start().expect("Engine start failed");
        let _event_rx = engine.subscribe_events();

        let ctx_provider =
            Arc::new(EguiWgpuContextProvider::from_eframe(cc).expect("Failed to create EguiWgpuContextProvider"));

        // Setup zone
        let zone_cfg = ZoneConfig::builder()
            .do_not_track(true)
            .accept_languages("fr-CH, fr;q=0.9, en;q=0.8, de;q=0.7, *;q=0.5")
            .build()
            .expect("ZoneConfig is not valid");

        let sqlite_store = SqliteCookieStore::new(".gosub-gtk-cookie-store.db".into());
        let cookie_store: gosub_engine_api::cookies::CookieStoreHandle = sqlite_store.into();

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
                zone.create_tab(
                    TabDefaults {
                        url: None,
                        title: Some("New Tab".to_string()),
                        viewport: Some(Viewport::new(0, 0, DEFAULT_WIDTH, DEFAULT_HEIGHT)),
                    },
                    None,
                )
                .await
            })
            .expect("create_tab failed");

        let tab_id = tab.tab_id;
        tabs.insert(tab_id, tab);

        let active_tab = tab_id;
        let root: LayoutHandle = std::rc::Rc::new(std::cell::RefCell::new(LayoutNode::Leaf(active_tab)));

        Self {
            engine,
            zone,
            root,
            active_tab,
            tabs,
            last_size: (DEFAULT_WIDTH as i32, DEFAULT_HEIGHT as i32),
            compositor,
            current_url_input: String::new(),
            needs_redraw: true,
            pointer_pos: (0.0, 0.0),
            backend_initialized: false,
            ctx_provider,
            last_panel_size: egui::Vec2::ZERO,
        }
    }

    fn handle_navigation(&mut self) {
        let composed_url = self.current_url_input.clone();

        let url_str = if !composed_url.starts_with("http://") && !composed_url.starts_with("https://") {
            format!("https://{}", composed_url)
        } else {
            composed_url
        };

        let Ok(url) = Url::parse(&url_str) else {
            return;
        };

        let tab_id = self.active_tab;
        if let Some(tab) = self.tabs.get(&tab_id).cloned() {
            TOKIO_RT.spawn(async move {
                let _ = tab.navigate(url.as_str()).await;
            });
        }
        self.needs_redraw = true;
    }

    fn handle_split_col(&mut self) {
        let (w, h) = self.last_size;
        let new_tab = TOKIO_RT
            .block_on(self.zone.create_tab(
                TabDefaults {
                    url: None,
                    title: Some("New Tab".to_string()),
                    viewport: Some(Viewport::new(0, 0, (w / 2).max(1) as u32, h as u32)),
                },
                None,
            ))
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
        self.needs_redraw = true;
    }

    fn handle_split_row(&mut self) {
        let (w, h) = self.last_size;
        let new_tab = TOKIO_RT
            .block_on(self.zone.create_tab(
                TabDefaults {
                    url: None,
                    title: Some("New Tab".to_string()),
                    viewport: Some(Viewport::new(0, 0, w as u32, (h / 2).max(1) as u32)),
                },
                None,
            ))
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
        self.needs_redraw = true;
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
            self.needs_redraw = true;
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
            self.needs_redraw = true;
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
            self.needs_redraw = true;
        }
    }

    fn init_vello_backend(&mut self) {
        if self.backend_initialized {
            return;
        }
        // TODO: swap the null backend for a real Vello backend once the engine supports
        // replacing the backend after construction.
        self.backend_initialized = true;
        self.needs_redraw = true;
    }
}

impl eframe::App for GosubApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        ctx.request_repaint_after(std::time::Duration::from_millis(16));

        self.init_vello_backend();

        let mut style = (*ctx.style()).clone();
        style.text_styles.get_mut(&egui::TextStyle::Body).unwrap().size = 16.0;
        style.text_styles.get_mut(&egui::TextStyle::Button).unwrap().size = 14.0;
        ctx.set_style(style);

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

        // Drain engine events to check for redraws / page loads
        let mut event_rx = self.engine.subscribe_events();
        while let Ok(ev) = event_rx.try_recv() {
            if let EngineEvent::Navigation {
                event: NavigationEvent::Finished { .. },
                ..
            } = ev
            {
                self.needs_redraw = true;
            }
        }

        egui::TopBottomPanel::top("address_bar")
            .frame(egui::Frame::default().inner_margin(egui::Margin::symmetric(12, 10)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let response = ui.add(
                        egui::TextEdit::singleline(&mut self.current_url_input)
                            .desired_width(f32::INFINITY)
                            .frame(true)
                            .hint_text("Enter URL")
                            .char_limit(100)
                            .font(egui::TextStyle::Heading)
                            .min_size(egui::vec2(0.0, 36.0)),
                    );

                    if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        self.handle_navigation();
                    }
                });
            });

        egui::TopBottomPanel::top("toolbar")
            .frame(egui::Frame::default().inner_margin(egui::Margin::symmetric(12, 8)))
            .show(ctx, |ui| {
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

        let panel = egui::CentralPanel::default().show(ctx, |ui| {
            let available_size = ui.available_size();
            self.last_size = (available_size.x as i32, available_size.y as i32);

            let (w, h) = self.last_size;
            let mut pairs = Vec::new();
            compute_layout(&self.root.borrow(), Rect { x: 0, y: 0, w, h }, &mut pairs);

            let active_tab_id = self.active_tab;

            for (tab_id, rect) in pairs {
                let is_active = tab_id == active_tab_id;

                if let Some(handle) = self.compositor.read().unwrap().frame_for(tab_id) {
                    let rect_ui = egui::Rect::from_min_max(
                        egui::pos2(rect.x as f32, rect.y as f32),
                        egui::pos2(rect.w as f32, rect.h as f32),
                    );

                    match handle {
                        ExternalHandle::WgpuTextureId { id, width, height, .. } => {
                            let (_, view) = self.ctx_provider.get_texture(id).unwrap();

                            let mut renderer = frame.wgpu_render_state().unwrap().renderer.write();
                            let device = &frame.wgpu_render_state().unwrap().device;

                            let tid =
                                renderer.register_native_texture(device, &view, eframe::wgpu::FilterMode::Nearest);

                            let size_points = egui::Vec2::new((width - 25) as f32, (height - 25) as f32);
                            ui.add(egui::Image::new(SizedTexture::new(tid, size_points)));
                        }
                        _ => {
                            eprintln!("Unsupported handle type for tab {:?}: {:?}", tab_id, handle);
                        }
                    }

                    let col = if is_active {
                        egui::Color32::YELLOW
                    } else {
                        egui::Color32::DARK_GRAY
                    };

                    ui.painter()
                        .rect_stroke(rect_ui, 0.0, egui::Stroke::new(1.0, col), StrokeKind::Outside);
                }
            }

            if self.needs_redraw {
                ctx.request_repaint();
                self.needs_redraw = false;
            }
        });

        let size = panel.response.rect.size();
        if size != self.last_panel_size {
            self.last_panel_size = size;

            let mut pairs = Vec::new();
            compute_layout(
                &self.root.borrow(),
                Rect {
                    x: 0,
                    y: 0,
                    w: size.x as i32,
                    h: size.y as i32,
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
    }
}

fn main() -> Result<(), eframe::Error> {
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
