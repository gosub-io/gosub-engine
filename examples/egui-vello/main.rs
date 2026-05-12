use crate::tiling::{
    close_leaf, collect_leaves, compute_layout, find_leaf_at, split_leaf_into_cols, split_leaf_into_rows, LayoutHandle,
    LayoutNode, Rect,
};
use eframe::{egui, CreationContext};
use egui::StrokeKind;
use gosub_engine::cookies::SqliteCookieStore;
use gosub_engine::events::{EngineEvent, NavigationEvent, TabCommand};
use gosub_engine::render::backends::null::NullBackend;
use gosub_engine::render::{DefaultCompositor, Viewport};
use gosub_engine::storage::{InMemorySessionStore, PartitionPolicy, SqliteLocalStore, StorageService};
use gosub_engine::tab::{TabDefaults, TabHandle, TabId};
use gosub_engine::zone::{Zone, ZoneConfig, ZoneId, ZoneServices};
use gosub_engine::GosubEngine;
use once_cell::sync::Lazy;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::runtime::{Builder, Runtime};
use url::Url;
use uuid::uuid;

mod tiling;

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
    #[allow(dead_code)]
    engine: GosubEngine,
    zone: Zone,
    root: LayoutHandle,
    active_tab: TabId,
    tabs: HashMap<TabId, TabHandle>,
    last_size: (i32, i32),
    current_url_input: String,
    needs_redraw: bool,
    pointer_pos: (f64, f64),
    last_panel_size: egui::Vec2,
    event_rx: tokio::sync::broadcast::Receiver<EngineEvent>,
}

impl GosubApp {
    fn new(_cc: &CreationContext) -> Self {
        // Enter Tokio so any internal tokio::spawn/time/io in engine works
        let _rt_guard = TOKIO_RT.enter();

        let backend = Arc::new(NullBackend::new().expect("NullBackend::new failed"));
        let compositor = Arc::new(RwLock::new(DefaultCompositor::default()));

        let mut engine = GosubEngine::new(None, backend, compositor);
        let _engine_join_handle = engine.start().expect("Engine start failed");
        let event_rx = engine.subscribe_events();

        // Setup zone
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
            needs_redraw: true,
            pointer_pos: (0.0, 0.0),
            last_panel_size: egui::Vec2::ZERO,
            event_rx,
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

        let tab_id = self.active_tab;
        if let Some(tab) = self.tabs.get(&tab_id).cloned() {
            TOKIO_RT.spawn(async move {
                let _ = tab.navigate(url.as_str()).await;
                let _ = tab.send(TabCommand::ResumeDrawing { fps: 30 }).await;
                ctx.request_repaint();
            });
        }
        self.needs_redraw = true;
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
        self.needs_redraw = true;
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
}

impl eframe::App for GosubApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        ctx.request_repaint_after(std::time::Duration::from_millis(16));

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

        // Drain engine events to check for redraws / page loads
        while let Ok(ev) = self.event_rx.try_recv() {
            if let EngineEvent::Navigation {
                event: NavigationEvent::Finished { .. },
                ..
            } = ev
            {
                self.needs_redraw = true;
                ctx.request_repaint();
            }
        }

        egui::Panel::top("address_bar")
            .frame(egui::Frame::default().inner_margin(egui::Margin::symmetric(12, 10)))
            .show_inside(ui, |ui| {
                ui.horizontal(|ui| {
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

        let panel = egui::CentralPanel::default().show_inside(ui, |ui| {
            let available_size = ui.available_size();
            self.last_size = (available_size.x as i32, available_size.y as i32);

            let (w, h) = self.last_size;
            let mut pairs = Vec::new();
            compute_layout(&self.root.borrow(), Rect { x: 0, y: 0, w, h }, &mut pairs);

            let active_tab_id = self.active_tab;

            for (tab_id, rect) in pairs {
                let is_active = tab_id == active_tab_id;

                let rect_ui = egui::Rect::from_min_max(
                    egui::pos2(rect.x as f32, rect.y as f32),
                    egui::pos2((rect.x + rect.w) as f32, (rect.y + rect.h) as f32),
                );

                let source = self.tabs.get(&tab_id).and_then(|t| t.sink.source_html.read().clone());

                ui.painter().rect_filled(rect_ui, 0.0, egui::Color32::WHITE);
                ui.scope_builder(egui::UiBuilder::new().max_rect(rect_ui), |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt(tab_id)
                        .max_height(rect_ui.height())
                        .show(ui, |ui| {
                            if let Some(html) = source {
                                ui.add(egui::Label::new(egui::RichText::new(html).monospace().size(12.0)).wrap());
                            } else {
                                ui.label(
                                    egui::RichText::new("No page loaded")
                                        .italics()
                                        .color(egui::Color32::GRAY),
                                );
                            }
                        });
                });

                let col = if is_active {
                    egui::Color32::YELLOW
                } else {
                    egui::Color32::DARK_GRAY
                };
                ui.painter()
                    .rect_stroke(rect_ui, 0.0, egui::Stroke::new(2.0, col), StrokeKind::Outside);
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
