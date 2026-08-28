//! A throwaway mini browser for watching process isolation work.
//!
//! Every isolation setting is on: the network stack runs in its own sandboxed
//! process, each image decodes in a throwaway process, and every page is
//! rendered by a resident renderer (one per site, forked from the fork server
//! and shared by that site's tabs) — parse, style, layout, paint and
//! rasterization all happen out of process, and the tiles come back as sealed
//! shared memory. What this window composites was never produced in
//! this process.
//!
//! Run it:
//!
//! ```sh
//! cargo run -p example-mini-browser -- https://example.com https://gosub.io
//! ```
//!
//! Storage and cookies are out of process too: the zone's localStorage is a
//! `FileLocalStore` under `~/.cache/gosub-mini-browser` (served by
//! `gosub-storage`), its cookie jar lives in `gosub-vault` and persists through
//! a SQLite store there. With no scripts to touch localStorage, the browser
//! stands in for one: every finished navigation bumps a per-origin `visits`
//! counter and prints it, with the cookie count the vault reports.
//!
//! Keys: Ctrl+T new tab, Ctrl+W close tab (middle-click a tab also closes it),
//! Ctrl+L focus the address bar, F5/Ctrl+R reload, Ctrl+P print the renderer
//! pool and the live process tree to the terminal. Or watch from outside:
//!
//! ```sh
//! pstree -ap <broker pid>    # printed at startup
//! ```
//!
//! The embedder's own render backend is the null backend on purpose: if the
//! isolated path broke and the engine fell back to in-process rendering, tabs
//! would go blank rather than quietly rendering un-isolated.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use gosub_config::settings::Setting;
use gosub_engine::cookies::{CookieStoreHandle, SqliteCookieStore};
use gosub_engine::events::{EngineEvent, MouseButton as GosubMouseButton, NavigationEvent, TabCommand};
use gosub_engine::storage::{FileLocalStore, InMemorySessionStore, PartitionKey, PartitionPolicy, StorageService};
use gosub_engine::tab::{TabHandle, TabId};
use gosub_engine::zone::{Zone, ZoneServices};
use gosub_engine::GosubEngine;
use gosub_render_pipeline::render::backend::ExternalHandle;
use gosub_render_pipeline::render::backends::null::NullBackend;
use gosub_render_pipeline::render::{composite_tiles, DefaultCompositor, TileTarget, DEVICE_PIXEL_RATIO};
use once_cell::sync::Lazy;
use softbuffer::Surface;
use std::num::NonZeroU32;
use std::sync::Arc;
use tokio::runtime::{Builder, Runtime};
use url::Url;
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, KeyEvent, Modifiers, MouseButton as WinitMouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowAttributes, WindowId};

const TAB_BAR_HEIGHT: u32 = 30;
const ADDRESS_BAR_HEIGHT: u32 = 36;
const CHROME_HEIGHT: u32 = TAB_BAR_HEIGHT + ADDRESS_BAR_HEIGHT;
const TAB_MAX_WIDTH: u32 = 220;
const NEW_TAB_BUTTON_WIDTH: u32 = 30;
const SCROLL_MULTIPLIER: f32 = 134.0;

// ── Render configuration ─────────────────────────────────────────────────────
//
// `DefaultRenderConfig` ships no forked-tile rasterizer, so a config of our own
// supplies one: the Cairo CPU rasterizer, running inside the forked renderer
// under seccomp. Parley shapes text without touching font files at shape time,
// which is what lets the fork server warm fonts once and run renderers at the
// `Full` confinement tier.

#[derive(Clone, Debug, PartialEq)]
struct MiniConfig;

impl gosub_interface::config::ModuleConfiguration for MiniConfig {
    type CssSystem = gosub_css3::system::Css3System;
    type Document = gosub_html5::document::document_impl::DocumentImpl<Self>;
    type HtmlParser = gosub_html5::parser::Html5Parser<'static, Self>;
}

impl gosub_engine::html::RenderConfiguration for MiniConfig {
    type RenderBackend = NullBackend;
    type CompositorSink = DefaultCompositor;
    type FontSystem = gosub_fontmanager::ParleyFontSystem;

    fn forked_tile_rasterizer(
        font_system: Arc<parking_lot::Mutex<dyn gosub_interface::font_system::FontSystem>>,
    ) -> Option<Box<dyn gosub_render_pipeline::rasterizer::Rasterable + Send + Sync>> {
        Some(Box::new(gosub_renderer_cairo::CairoRasterizer::with_font_system(
            font_system,
        )))
    }
}

static TOKIO_RT: Lazy<Runtime> = Lazy::new(|| {
    Builder::new_multi_thread()
        .enable_io()
        .enable_time()
        .thread_name("gosub-mini-browser-rt")
        .build()
        .expect("tokio runtime")
});

// ── UI events (engine → winit thread) ────────────────────────────────────────

#[derive(Debug)]
enum UiEvent {
    /// The compositor holds a new frame for some tab.
    Frame,
    NavStarted {
        tab: TabId,
        url: String,
    },
    NavFinished {
        tab: TabId,
        url: String,
    },
    NavFailed {
        tab: TabId,
        error: String,
    },
    TitleChanged {
        tab: TabId,
        title: String,
    },
}

// ── Tab state ────────────────────────────────────────────────────────────────

struct TabState {
    handle: TabHandle,
    id: TabId,
    url_input: String,
    label: String,
    loading: bool,
    scroll: (f32, f32),
    page_height: f32,
}

impl TabState {
    fn new(handle: TabHandle, url: &str) -> Self {
        let label = host_of(url).unwrap_or_else(|| "new tab".into());
        Self {
            id: handle.tab_id,
            handle,
            url_input: url.to_string(),
            label,
            loading: !url.is_empty(),
            scroll: (0.0, 0.0),
            page_height: 0.0,
        }
    }
}

fn host_of(url: &str) -> Option<String> {
    Url::parse(url).ok()?.host_str().map(str::to_string)
}

// ── The application ──────────────────────────────────────────────────────────

struct BrowserApp {
    engine: GosubEngine<MiniConfig>,
    zone: Zone<MiniConfig>,
    compositor: Arc<DefaultCompositor>,
    /// The zone's storage, to play the role of a script using localStorage.
    storage: Arc<StorageService>,
    cookie_store: Option<CookieStoreHandle>,

    tabs: Vec<TabState>,
    active: usize,

    window: Option<Arc<Window>>,
    surface: Option<Surface<Arc<Window>, Arc<Window>>>,
    surface_size: (u32, u32),

    addr_focused: bool,
    cursor: PhysicalPosition<f64>,
    modifiers: Modifiers,
}

impl BrowserApp {
    /// What a page script would do with localStorage: count this origin's
    /// visits. The area lives in the storage process; the jar count comes from
    /// the store the vault snapshots into.
    fn note_visit(&self, url: &str) {
        let Some(origin) = Url::parse(url).ok().map(|u| u.origin()) else {
            return;
        };
        let Ok(area) = self.storage.local_for(self.zone.id, &PartitionKey::None, &origin) else {
            return;
        };
        let visits = area.get_item("visits").and_then(|v| v.parse::<u64>().ok()).unwrap_or(0) + 1;
        if let Err(e) = area.set_item("visits", &visits.to_string()) {
            println!(
                "[storage] {} could not record the visit: {e}",
                origin.ascii_serialization()
            );
            return;
        }
        let cookies = self
            .cookie_store
            .as_ref()
            .and_then(|store| store.jar_for(self.zone.id))
            .map(|jar| jar.read().get_all_cookies().len())
            .unwrap_or(0);
        println!(
            "[storage] {}: visit #{visits} (localStorage in gosub-storage); {cookies} cookie origin(s) in the vault",
            origin.ascii_serialization()
        );
    }

    fn active_tab(&mut self) -> Option<&mut TabState> {
        self.tabs.get_mut(self.active)
    }

    fn tab_index(&self, id: TabId) -> Option<usize> {
        self.tabs.iter().position(|t| t.id == id)
    }

    fn content_size(&self) -> (u32, u32) {
        let (w, h) = self.surface_size;
        (w, h.saturating_sub(CHROME_HEIGHT))
    }

    fn send_viewport(&self, tab: &TabState) {
        let (w, h) = self.content_size();
        if w == 0 || h == 0 {
            return;
        }
        let handle = tab.handle.clone();
        TOKIO_RT.spawn(async move {
            let _ = handle
                .send(TabCommand::SetViewport {
                    x: 0,
                    y: 0,
                    width: w,
                    height: h,
                })
                .await;
            let _ = handle.send(TabCommand::ResumeDrawing { fps: 30 }).await;
        });
    }

    fn new_tab(&mut self, url: Option<String>) {
        let handle = match TOKIO_RT.block_on(self.zone.create_tab(Default::default(), None)) {
            Ok(h) => h,
            Err(e) => {
                log::error!("create_tab failed: {e}");
                return;
            }
        };
        let url = url.unwrap_or_default();
        let tab = TabState::new(handle, &url);
        self.send_viewport(&tab);
        if !url.is_empty() {
            let handle = tab.handle.clone();
            let url = url.clone();
            TOKIO_RT.spawn(async move {
                let _ = handle.navigate(url).await;
            });
        }
        self.tabs.push(tab);
        self.active = self.tabs.len() - 1;
        self.addr_focused = url.is_empty();
        self.request_redraw();
    }

    fn close_tab(&mut self, index: usize, event_loop: &ActiveEventLoop) {
        if index >= self.tabs.len() {
            return;
        }
        let tab = self.tabs.remove(index);
        TOKIO_RT.block_on(self.zone.close_tab(tab.id));
        if self.tabs.is_empty() {
            event_loop.exit();
            return;
        }
        if self.active >= self.tabs.len() {
            self.active = self.tabs.len() - 1;
        }
        self.request_redraw();
    }

    fn navigate(&mut self) {
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return;
        };
        let mut s = tab.url_input.clone();
        if !s.starts_with("http://") && !s.starts_with("https://") {
            s = format!("https://{s}");
            tab.url_input = s.clone();
        }
        if Url::parse(&s).is_err() {
            return;
        }
        tab.scroll = (0.0, 0.0);
        tab.loading = true;
        tab.label = host_of(&s).unwrap_or_else(|| s.clone());
        self.addr_focused = false;
        let handle = tab.handle.clone();
        TOKIO_RT.spawn(async move {
            let _ = handle.navigate(s).await;
            let _ = handle.send(TabCommand::ResumeDrawing { fps: 30 }).await;
        });
        self.request_redraw();
    }

    fn reload(&mut self) {
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return;
        };
        tab.loading = true;
        let handle = tab.handle.clone();
        TOKIO_RT.spawn(async move {
            let _ = handle.send(TabCommand::Reload { ignore_cache: false }).await;
        });
        self.request_redraw();
    }

    fn request_redraw(&self) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    // ── geometry ──

    fn tab_strip_width(&self, win_w: u32) -> u32 {
        let n = self.tabs.len().max(1) as u32;
        let avail = win_w.saturating_sub(NEW_TAB_BUTTON_WIDTH);
        (avail / n).min(TAB_MAX_WIDTH)
    }

    fn tab_at(&self, x: f64) -> Option<usize> {
        let w = self.tab_strip_width(self.surface_size.0) as f64;
        if w <= 0.0 {
            return None;
        }
        let i = (x / w) as usize;
        (i < self.tabs.len() && x >= 0.0).then_some(i)
    }

    fn in_new_tab_button(&self, x: f64) -> bool {
        let strip_end = self.tab_strip_width(self.surface_size.0) as f64 * self.tabs.len() as f64;
        x >= strip_end && x < strip_end + NEW_TAB_BUTTON_WIDTH as f64
    }

    fn content_x_to_css(&self, physical_x: f64, tab: &TabState) -> f32 {
        let dpr = DEVICE_PIXEL_RATIO.load(std::sync::atomic::Ordering::Relaxed) as f64;
        (physical_x / dpr + tab.scroll.0 as f64) as f32
    }

    fn content_y_to_css(&self, physical_y: f64, tab: &TabState) -> f32 {
        let dpr = DEVICE_PIXEL_RATIO.load(std::sync::atomic::Ordering::Relaxed) as f64;
        let logical_y = physical_y / dpr - CHROME_HEIGHT as f64;
        (logical_y + tab.scroll.1 as f64) as f32
    }

    // ── drawing ──

    fn redraw(&mut self) {
        let (win_w, win_h) = self.surface_size;
        let tab_w = self.tab_strip_width(win_w);
        let Some(surface) = &mut self.surface else { return };
        if win_w == 0 || win_h == 0 {
            return;
        }
        let (Ok(nw), Ok(nh)) = (NonZeroU32::try_from(win_w), NonZeroU32::try_from(win_h)) else {
            return;
        };
        if surface.resize(nw, nh).is_err() {
            return;
        }
        let Ok(mut buf) = surface.buffer_mut() else { return };
        buf.fill(0xFFFF_FFFF);

        let content_h = win_h.saturating_sub(CHROME_HEIGHT);
        let active = self.active;
        if content_h > 0 {
            if let Some(tab) = self.tabs.get_mut(active) {
                if let Some(ExternalHandle::TileCache {
                    tiles,
                    dpr: tile_dpr,
                    page_height,
                    scroll_x,
                    scroll_y,
                    ..
                }) = self.compositor.frame_for(tab.id)
                {
                    tab.page_height = page_height;
                    composite_tiles(
                        &tiles,
                        tile_dpr,
                        (scroll_x, scroll_y),
                        &mut TileTarget {
                            buf: &mut buf[..],
                            stride: win_w as usize,
                            origin_x: 0,
                            origin_y: CHROME_HEIGHT as usize,
                            width: win_w as usize,
                            height: content_h as usize,
                        },
                    );
                }
            }
        }

        draw_chrome(&mut buf, win_w, &self.tabs, self.active, tab_w, self.addr_focused);

        buf.present().unwrap_or_default();
    }
}

impl ApplicationHandler<UiEvent> for BrowserApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attrs = WindowAttributes::default()
            .with_title("Gosub mini browser — process isolated")
            .with_inner_size(LogicalSize::new(1024u32, 768u32));
        let window = Arc::new(event_loop.create_window(attrs).expect("create window"));

        let dpr = window.scale_factor() as u32;
        DEVICE_PIXEL_RATIO.store(dpr.max(1), std::sync::atomic::Ordering::Relaxed);

        let ctx = softbuffer::Context::new(window.clone()).expect("softbuffer context");
        let surface = Surface::new(&ctx, window.clone()).expect("softbuffer surface");
        let size = window.inner_size();
        self.surface_size = (size.width, size.height);
        self.window = Some(window);
        self.surface = Some(surface);

        for tab in &self.tabs {
            self.send_viewport(tab);
        }
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: UiEvent) {
        match event {
            UiEvent::Frame => {}
            UiEvent::NavStarted { tab, url } => {
                if let Some(i) = self.tab_index(tab) {
                    let t = &mut self.tabs[i];
                    t.loading = true;
                    t.label = host_of(&url).unwrap_or(url);
                }
            }
            UiEvent::NavFinished { tab, url } => {
                self.note_visit(&url);
                if let Some(i) = self.tab_index(tab) {
                    let t = &mut self.tabs[i];
                    t.loading = false;
                    if !(self.addr_focused && i == self.active) {
                        t.url_input = url;
                    }
                }
            }
            UiEvent::NavFailed { tab, error } => {
                if let Some(i) = self.tab_index(tab) {
                    let t = &mut self.tabs[i];
                    t.loading = false;
                    t.label = "load failed".into();
                    log::warn!("tab {}: navigation failed: {error}", t.id);
                }
            }
            UiEvent::TitleChanged { tab, title } => {
                if let Some(i) = self.tab_index(tab) {
                    self.tabs[i].label = title;
                }
            }
        }
        self.request_redraw();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::RedrawRequested => self.redraw(),
            WindowEvent::ModifiersChanged(m) => self.modifiers = m,

            WindowEvent::Resized(PhysicalSize { width, height }) => {
                if width == 0 || height == 0 {
                    return;
                }
                self.surface_size = (width, height);
                let dpr = self.window.as_ref().map(|w| w.scale_factor() as u32).unwrap_or(1);
                DEVICE_PIXEL_RATIO.store(dpr.max(1), std::sync::atomic::Ordering::Relaxed);
                for tab in &self.tabs {
                    self.send_viewport(tab);
                }
                self.request_redraw();
            }

            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = position;
                if position.y >= CHROME_HEIGHT as f64 {
                    if let Some(tab) = self.tabs.get(self.active) {
                        let x = self.content_x_to_css(position.x, tab);
                        let y = self.content_y_to_css(position.y, tab);
                        let handle = tab.handle.clone();
                        TOKIO_RT.spawn(async move {
                            let _ = handle.send(TabCommand::MouseMove { x, y }).await;
                        });
                    }
                }
            }

            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button,
                ..
            } => {
                let pos = self.cursor;
                if pos.y < TAB_BAR_HEIGHT as f64 {
                    match button {
                        WinitMouseButton::Left => {
                            if let Some(i) = self.tab_at(pos.x) {
                                self.active = i;
                                self.addr_focused = false;
                            } else if self.in_new_tab_button(pos.x) {
                                self.new_tab(None);
                            }
                        }
                        WinitMouseButton::Middle => {
                            if let Some(i) = self.tab_at(pos.x) {
                                self.close_tab(i, event_loop);
                            }
                        }
                        _ => {}
                    }
                    self.request_redraw();
                } else if pos.y < CHROME_HEIGHT as f64 {
                    if button == WinitMouseButton::Left {
                        self.addr_focused = true;
                        self.request_redraw();
                    }
                } else if button == WinitMouseButton::Left {
                    self.addr_focused = false;
                    if let Some(tab) = self.tabs.get(self.active) {
                        let x = self.content_x_to_css(pos.x, tab);
                        let y = self.content_y_to_css(pos.y, tab);
                        let handle = tab.handle.clone();
                        TOKIO_RT.spawn(async move {
                            let _ = handle
                                .send(TabCommand::MouseDown {
                                    x,
                                    y,
                                    button: GosubMouseButton::Left,
                                })
                                .await;
                        });
                    }
                    self.request_redraw();
                }
            }

            WindowEvent::MouseWheel { delta, .. } => {
                let (dx, dy) = match delta {
                    MouseScrollDelta::LineDelta(x, y) => (x * SCROLL_MULTIPLIER, y * SCROLL_MULTIPLIER),
                    MouseScrollDelta::PixelDelta(p) => (p.x as f32, p.y as f32),
                };
                let (_, content_h) = self.content_size();
                if let Some(tab) = self.tabs.get_mut(self.active) {
                    let max_y = (tab.page_height - content_h as f32).max(0.0);
                    tab.scroll.0 = (tab.scroll.0 + dx).max(0.0);
                    tab.scroll.1 = (tab.scroll.1 + dy).clamp(0.0, max_y);
                    let handle = tab.handle.clone();
                    TOKIO_RT.spawn(async move {
                        let _ = handle
                            .send(TabCommand::MouseScroll {
                                delta_x: dx,
                                delta_y: dy,
                            })
                            .await;
                    });
                }
                self.request_redraw();
            }

            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        logical_key,
                        text,
                        state: ElementState::Pressed,
                        ..
                    },
                ..
            } => {
                let ctrl = self.modifiers.state().control_key();
                if ctrl {
                    match &logical_key {
                        Key::Character(c) if c == "t" => self.new_tab(None),
                        Key::Character(c) if c == "w" => self.close_tab(self.active, event_loop),
                        Key::Character(c) if c == "l" => {
                            self.addr_focused = true;
                            if let Some(tab) = self.active_tab() {
                                tab.url_input.clear();
                            }
                            self.request_redraw();
                        }
                        Key::Character(c) if c == "r" => self.reload(),
                        Key::Character(c) if c == "p" => {
                            self.print_renderers();
                            print_process_tree();
                        }
                        _ => {}
                    }
                    return;
                }
                if logical_key == Key::Named(NamedKey::F5) {
                    self.reload();
                    return;
                }
                if !self.addr_focused {
                    return;
                }
                match &logical_key {
                    Key::Named(NamedKey::Enter) => self.navigate(),
                    Key::Named(NamedKey::Escape) => {
                        self.addr_focused = false;
                        self.request_redraw();
                    }
                    Key::Named(NamedKey::Backspace) => {
                        if let Some(tab) = self.active_tab() {
                            tab.url_input.pop();
                        }
                        self.request_redraw();
                    }
                    _ => {
                        if let Some(t) = &text {
                            let t = t.as_str();
                            if !t.chars().any(char::is_control) {
                                if let Some(tab) = self.active_tab() {
                                    tab.url_input.push_str(t);
                                }
                                self.request_redraw();
                            }
                        }
                    }
                }
            }

            _ => {}
        }
    }
}

// ── Chrome drawing (plain Cairo, toy text API — no GTK, no Pango) ────────────

fn draw_chrome(
    buf: &mut softbuffer::Buffer<Arc<Window>, Arc<Window>>,
    win_w: u32,
    tabs: &[TabState],
    active: usize,
    tab_w: u32,
    addr_focused: bool,
) {
    let w = win_w as i32;
    let h = CHROME_HEIGHT as i32;
    let Ok(mut surface) = cairo::ImageSurface::create(cairo::Format::ARgb32, w, h) else {
        return;
    };
    {
        let Ok(cr) = cairo::Context::new(&surface) else { return };
        cr.select_font_face("Sans", cairo::FontSlant::Normal, cairo::FontWeight::Normal);

        // Tab strip.
        cr.set_source_rgb(0.80, 0.80, 0.82);
        cr.rectangle(0.0, 0.0, w as f64, TAB_BAR_HEIGHT as f64);
        cr.fill().unwrap_or_default();

        for (i, tab) in tabs.iter().enumerate() {
            let x = (i as u32 * tab_w) as f64;
            let is_active = i == active;
            if is_active {
                cr.set_source_rgb(0.95, 0.95, 0.96);
            } else {
                cr.set_source_rgb(0.87, 0.87, 0.89);
            }
            cr.rectangle(x + 1.0, 2.0, tab_w as f64 - 2.0, TAB_BAR_HEIGHT as f64 - 2.0);
            cr.fill().unwrap_or_default();

            cr.set_source_rgb(0.1, 0.1, 0.1);
            cr.set_font_size(12.0);
            cr.move_to(x + 8.0, TAB_BAR_HEIGHT as f64 - 10.0);
            let mut label = String::new();
            if tab.loading {
                label.push_str("* ");
            }
            label.push_str(&tab.label);
            let max_chars = ((tab_w as usize).saturating_sub(16)) / 7;
            if label.len() > max_chars && max_chars > 1 {
                label.truncate(max_chars - 1);
                label.push('…');
            }
            cr.show_text(&label).unwrap_or_default();
        }

        // New-tab button.
        let plus_x = (tabs.len() as u32 * tab_w) as f64;
        cr.set_source_rgb(0.3, 0.3, 0.3);
        cr.set_font_size(16.0);
        cr.move_to(plus_x + 10.0, TAB_BAR_HEIGHT as f64 - 9.0);
        cr.show_text("+").unwrap_or_default();

        // Address bar.
        let top = TAB_BAR_HEIGHT as f64;
        cr.set_source_rgb(0.93, 0.93, 0.93);
        cr.rectangle(0.0, top, w as f64, ADDRESS_BAR_HEIGHT as f64);
        cr.fill().unwrap_or_default();

        let (bg_r, bg_g, bg_b) = if addr_focused {
            (1.0, 1.0, 1.0)
        } else {
            (0.97, 0.97, 0.97)
        };
        cr.set_source_rgb(bg_r, bg_g, bg_b);
        cr.rectangle(4.0, top + 5.0, (w - 8) as f64, ADDRESS_BAR_HEIGHT as f64 - 10.0);
        cr.fill().unwrap_or_default();

        let (br, bg, bb) = if addr_focused {
            (0.26, 0.52, 0.96)
        } else {
            (0.7, 0.7, 0.7)
        };
        cr.set_source_rgb(br, bg, bb);
        cr.set_line_width(1.0);
        cr.rectangle(4.5, top + 5.5, (w - 9) as f64, ADDRESS_BAR_HEIGHT as f64 - 11.0);
        cr.stroke().unwrap_or_default();

        cr.set_source_rgb(0.0, 0.0, 0.0);
        cr.set_font_size(14.0);
        cr.move_to(10.0, top + ADDRESS_BAR_HEIGHT as f64 - 12.0);
        let url = tabs.get(active).map(|t| t.url_input.as_str()).unwrap_or("");
        cr.show_text(url).unwrap_or_default();
    }
    surface.flush();

    let Ok(data) = surface.data() else { return };
    let stride = (w * 4) as usize;
    for row in 0..CHROME_HEIGHT as usize {
        for col in 0..win_w as usize {
            let off = row * stride + col * 4;
            if off + 3 >= data.len() {
                break;
            }
            let b = data[off] as u32;
            let g = data[off + 1] as u32;
            let r = data[off + 2] as u32;
            let idx = row * win_w as usize + col;
            if idx < buf.len() {
                buf[idx] = (r << 16) | (g << 8) | b;
            }
        }
    }
}

// ── Process tree (Linux: read /proc, no external tools) ──────────────────────

#[cfg(target_os = "linux")]
fn print_process_tree() {
    let me = std::process::id();
    // (pid, ppid, role or comm)
    let mut procs: Vec<(u32, u32, String)> = Vec::new();
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else {
            continue;
        };
        let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
            continue;
        };
        // Field 4 (ppid) sits after the parenthesized comm, which may itself
        // contain spaces — parse from the closing paren.
        let Some(close) = stat.rfind(')') else { continue };
        let comm = stat[stat.find('(').map(|i| i + 1).unwrap_or(0)..close].to_string();
        let rest: Vec<&str> = stat[close + 1..].split_whitespace().collect();
        let Some(ppid) = rest.get(1).and_then(|s| s.parse::<u32>().ok()) else {
            continue;
        };
        let role = std::fs::read_to_string(format!("/proc/{pid}/cmdline"))
            .ok()
            .and_then(|c| {
                let args: Vec<&str> = c.split('\0').collect();
                let at = args.iter().position(|a| *a == "--gosub-child-role")?;
                args.get(at + 1).map(|r| format!("--gosub-child-role {r}"))
            })
            .unwrap_or(comm);
        procs.push((pid, ppid, role));
    }

    fn print_children(procs: &[(u32, u32, String)], parent: u32, depth: usize) {
        for (pid, _, role) in procs.iter().filter(|(_, ppid, _)| *ppid == parent) {
            println!("{}{pid}  {role}", "  ".repeat(depth));
            print_children(procs, *pid, depth + 1);
        }
    }

    println!();
    println!("process tree — broker {me} (tabs and the DOM live here):");
    println!("{me}  broker");
    print_children(&procs, me, 1);
    println!();
}

impl BrowserApp {
    /// The engine's own view: which resident renderer serves which site, and
    /// how many tabs each hosts.
    fn print_renderers(&self) {
        println!();
        #[cfg(target_os = "linux")]
        if let Some(pool) = self.engine.renderer_pool() {
            let running = pool.snapshot();
            println!("resident renderers ({}), one per (zone, site):", running.len());
            for r in running {
                println!(
                    "  pid {} (inside the renderer pid namespace: {})  {}  {} tab(s)",
                    r.pid,
                    ns_pid(r.pid as u32).unwrap_or_else(|| "?".into()),
                    r.key.site,
                    r.tabs
                );
            }
            return;
        }
        println!("no resident renderer pool (renderer isolation is off or unavailable)");
    }
}

/// The pid a process has inside its innermost pid namespace, from the
/// kernel's own translation (`NSpid:` in `/proc/<pid>/status`, outermost
/// first). The fork server's children live in a namespace where the anchor
/// is PID 1 and renderers count up from there.
#[cfg(target_os = "linux")]
fn ns_pid(host_pid: u32) -> Option<String> {
    let status = std::fs::read_to_string(format!("/proc/{host_pid}/status")).ok()?;
    let line = status.lines().find(|l| l.starts_with("NSpid:"))?;
    line.split_whitespace().last().map(str::to_string)
}

#[cfg(not(target_os = "linux"))]
fn print_process_tree() {
    println!("process tree printing is implemented for Linux only");
}

// ── main ─────────────────────────────────────────────────────────────────────

fn main() {
    // Must come first: the engine starts each component by re-exec'ing this
    // binary with a role argument, and a child must reach its role before any
    // embedder startup runs. In the parent this returns immediately.
    gosub_engine::child_process::dispatch_with::<MiniConfig>();

    simple_logger::SimpleLogger::new()
        .with_level(log::LevelFilter::Info)
        .env()
        .init()
        .unwrap_or_default();

    let urls: Vec<String> = {
        let args: Vec<String> = std::env::args().skip(1).collect();
        if args.is_empty() {
            vec!["https://example.com/".into()]
        } else {
            args.into_iter()
                .map(|a| if a.contains("://") { a } else { format!("https://{a}") })
                .collect()
        }
    };

    let _rt_guard = TOKIO_RT.enter();

    let event_loop = EventLoop::<UiEvent>::with_user_event().build().expect("event loop");
    let proxy = event_loop.create_proxy();

    let compositor = Arc::new(DefaultCompositor::new({
        let p = proxy.clone();
        move || {
            let _ = p.send_event(UiEvent::Frame);
        }
    }));

    let mut engine: GosubEngine<MiniConfig> = GosubEngine::new(None, Arc::new(NullBackend::new()), compositor.clone());

    // Before start(): the engine reads these once as it comes up. Watch the log
    // for "network stack running in a separate, sandboxed process" and
    // "renderer fork server ready" — without them, isolation fell back.
    for key in [
        "security.network_process",
        "security.image_decoder_process",
        "security.renderer_process",
        "security.cookie_vault",
    ] {
        engine
            .settings()
            .set(key, Setting::Bool(true))
            .unwrap_or_else(|e| panic!("enable {key}: {e}"));
    }

    let mut event_rx = engine.subscribe_events();
    TOKIO_RT.spawn(engine.start().expect("engine start"));

    // Forward the engine events the UI cares about onto the winit thread.
    let nav_proxy = proxy.clone();
    TOKIO_RT.spawn(async move {
        loop {
            let event = match event_rx.recv().await {
                Ok(e) => e,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            };
            let ui = match event {
                EngineEvent::Navigation { tab_id, event } => match event {
                    NavigationEvent::Started { url, .. } => Some(UiEvent::NavStarted {
                        tab: tab_id,
                        url: url.to_string(),
                    }),
                    NavigationEvent::Finished { url, .. } => Some(UiEvent::NavFinished {
                        tab: tab_id,
                        url: url.to_string(),
                    }),
                    NavigationEvent::Failed { error, .. } => Some(UiEvent::NavFailed {
                        tab: tab_id,
                        error: error.to_string(),
                    }),
                    _ => None,
                },
                EngineEvent::TitleChanged { tab_id, title } => Some(UiEvent::TitleChanged { tab: tab_id, title }),
                _ => None,
            };
            if let Some(ui) = ui {
                let _ = nav_proxy.send_event(ui);
            }
        }
    });

    // Persistent, so a second run shows the counters and cookies survived.
    let data_dir = std::env::var_os("HOME")
        .map(|h| std::path::PathBuf::from(h).join(".cache").join("gosub-mini-browser"))
        .unwrap_or_else(|| std::env::temp_dir().join("gosub-mini-browser"));
    let local_store = FileLocalStore::open(data_dir.join("storage")).expect("storage directory");
    let cookie_store = SqliteCookieStore::new(data_dir.join("cookies.db"))
        .map(CookieStoreHandle::from)
        .map_err(|e| eprintln!("no persistent cookie store ({e}); cookies stay for this run only"))
        .ok();
    let storage = Arc::new(StorageService::new(
        Arc::new(local_store),
        Arc::new(InMemorySessionStore::new()),
    ));
    let services = ZoneServices {
        storage: Arc::clone(&storage),
        cookie_store: cookie_store.clone(),
        cookie_jar: None,
        partition_policy: PartitionPolicy::None,
        places: None,
    };
    let zone = engine.create_zone(None, services, None).expect("create zone");
    println!("storage and cookies under {}", data_dir.display());

    let mut app = BrowserApp {
        engine,
        zone,
        compositor,
        storage,
        cookie_store,
        tabs: Vec::new(),
        active: 0,
        window: None,
        surface: None,
        surface_size: (0, 0),
        addr_focused: false,
        cursor: PhysicalPosition::default(),
        modifiers: Modifiers::default(),
    };
    for url in &urls {
        app.new_tab(Some(url.clone()));
    }
    app.active = 0;

    println!();
    println!("broker pid {} — watch the tree from outside with:", std::process::id());
    println!("    pstree -ap {}", std::process::id());
    println!("or press Ctrl+P in the window to print it from inside.");
    println!(
        "Ctrl+T new tab · Ctrl+W close tab (middle-click too) · Ctrl+L address bar (Enter to go) · F5/Ctrl+R reload"
    );
    println!("firehose: curl -N localhost:9090/events   (NDJSON)   stats: localhost:9090/metrics   pool: localhost:9090/renderers");
    println!("viewer:   open tools/telemetry-viewer/index.html in a browser");
    println!();

    event_loop.run_app(&mut app).expect("event loop run");
}
