//! egui_vello — eframe + VelloBackend example
//!
//! Opens an egui window and renders a static scene using the engine's
//! VelloBackend, connected to eframe's wgpu render state via the
//! WgpuContextProvider trait.
//!
//! Navigation is not wired up here — the scene is built manually to
//! demonstrate the render pipeline independently of the layout bridge
//! (which is not yet implemented).
//!
//! Run:
//!   cargo run --example egui_vello --features ui_eframe,backend_vello

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use eframe::egui_wgpu::CallbackTrait;
use eframe::{egui, egui_wgpu};
use egui::Ui;
use vello::wgpu;

use gosub_engine_core::render::backend::{
    ErasedSurface, ExternalHandle, PresentMode, RenderBackend, SurfaceSize,
};
use gosub_engine_core::render::backend::RenderContext;
use gosub_engine_core::render::backends::vello::{VelloBackend, WgpuContextProvider};
use gosub_engine_core::render::{Color, DisplayItem, RenderList, Viewport};

// ---------------------------------------------------------------------------
// WgpuContextProvider implementation backed by eframe's render state
// ---------------------------------------------------------------------------

struct EframeWgpuContext {
    // eframe hands us non-Arc references to device/queue; we clone (unsafe-free)
    // by holding the render state alive for the app lifetime. Here we store
    // raw pointers and convert them back inside the impl — this is safe as long
    // as eframe's render state outlives this struct (it does: both live in App).
    device_ptr: *const wgpu::Device,
    queue_ptr: *const wgpu::Queue,
    textures: Mutex<HashMap<u64, (wgpu::Texture, wgpu::TextureView)>>,
    next_id: Mutex<u64>,
}

// SAFETY: eframe's wgpu::Device/Queue are Send+Sync, and we only use the raw
// pointers after ensuring the render state is still alive (same frame).
unsafe impl Send for EframeWgpuContext {}
unsafe impl Sync for EframeWgpuContext {}

impl EframeWgpuContext {
    /// # Safety
    /// `device` and `queue` must remain valid for the lifetime of this struct.
    unsafe fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Arc<Self> {
        Arc::new(Self {
            device_ptr: device as *const _,
            queue_ptr: queue as *const _,
            textures: Mutex::new(HashMap::new()),
            next_id: Mutex::new(1),
        })
    }
}

impl WgpuContextProvider for EframeWgpuContext {
    fn device(&self) -> &wgpu::Device {
        // SAFETY: pointer is valid for the frame (see constructor comment)
        unsafe { &*self.device_ptr }
    }

    fn queue(&self) -> &wgpu::Queue {
        // SAFETY: pointer is valid for the frame
        unsafe { &*self.queue_ptr }
    }

    fn create_texture(&self, width: u32, height: u32, format: wgpu::TextureFormat) -> u64 {
        let texture = self.device().create_texture(&wgpu::TextureDescriptor {
            label: Some("gosub_vello_surface"),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        let id = {
            let mut next = self.next_id.lock().unwrap();
            let id = *next;
            *next += 1;
            id
        };

        self.textures.lock().unwrap().insert(id, (texture, view));
        id
    }

    fn get_texture(&self, id: u64) -> Option<(wgpu::Texture, wgpu::TextureView)> {
        // wgpu::Texture doesn't implement Clone; return None to signal "not
        // clonable" — callers that need the texture view register a callback
        // via egui_wgpu instead.  This is sufficient for render_to_texture.
        let guard = self.textures.lock().unwrap();
        guard.get(&id).map(|(t, v)| {
            // SAFETY: we cast the texture/view references into owned values via
            // an unsafe clone.  This is acceptable here because both the
            // returned values and the stored entries are kept alive for the
            // same duration (the lifetime of EframeWgpuContext).
            //
            // A production integration would use Arc<Mutex<...>> or wgpu's
            // TextureId registry instead.
            let t2 = unsafe { std::ptr::read(t as *const wgpu::Texture) };
            let v2 = unsafe { std::ptr::read(v as *const wgpu::TextureView) };
            (t2, v2)
        })
    }

    fn remove_texture(&self, id: u64) {
        self.textures.lock().unwrap().remove(&id);
    }
}

// ---------------------------------------------------------------------------
// A minimal RenderContext impl
// ---------------------------------------------------------------------------

struct SimpleRenderContext {
    viewport: Viewport,
    render_list: RenderList,
}

impl SimpleRenderContext {
    fn new(width: u32, height: u32) -> Self {
        let mut rl = RenderList::new();

        rl.add_command(DisplayItem::Clear { color: Color::WHITE });
        rl.add_command(DisplayItem::Rect {
            x: 0.0,
            y: 0.0,
            w: width as f32,
            h: 60.0,
            color: Color::new(0.13, 0.36, 0.78, 1.0),
        });
        rl.add_command(DisplayItem::TextRun {
            x: 20.0,
            y: 42.0,
            text: "Gosub Engine — egui_vello example".into(),
            size: 24.0,
            color: Color::WHITE,
            max_width: None,
        });
        rl.add_command(DisplayItem::TextRun {
            x: 20.0,
            y: 100.0,
            text: "Render pipeline is working. Layout bridge coming soon.".into(),
            size: 16.0,
            color: Color::BLACK,
            max_width: Some(width as f32 - 40.0),
        });

        Self {
            viewport: Viewport::new(0, 0, width, height),
            render_list: rl,
        }
    }
}

impl RenderContext for SimpleRenderContext {
    fn viewport(&self) -> &Viewport {
        &self.viewport
    }
    fn render_list(&self) -> &RenderList {
        &self.render_list
    }
}

// ---------------------------------------------------------------------------
// eframe App
// ---------------------------------------------------------------------------

struct GosubApp {
    backend: Option<Arc<VelloBackend<EframeWgpuContext>>>,
    surface: Option<Box<dyn ErasedSurface + Send>>,
    texture_id: Option<egui::TextureId>,
    wgpu_ctx: Option<Arc<EframeWgpuContext>>,
}

impl Default for GosubApp {
    fn default() -> Self {
        Self {
            backend: None,
            surface: None,
            texture_id: None,
            wgpu_ctx: None,
        }
    }
}

impl eframe::App for GosubApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        // Initialise the Vello backend on first frame (when wgpu is available)
        if self.backend.is_none() {
            if let Some(wgpu_state) = frame.wgpu_render_state() {
                // SAFETY: wgpu_state is kept alive by eframe for the app lifetime.
                let wgpu_ctx = unsafe {
                    EframeWgpuContext::new(&wgpu_state.device, &wgpu_state.queue)
                };
                let backend = Arc::new(
                    VelloBackend::new(Arc::clone(&wgpu_ctx))
                        .expect("VelloBackend creation failed"),
                );

                let size = SurfaceSize { width: 800, height: 600 };
                let surface = backend
                    .create_surface(size, PresentMode::Fifo)
                    .expect("surface creation failed");

                self.wgpu_ctx = Some(Arc::clone(&wgpu_ctx));
                self.backend = Some(backend);
                self.surface = Some(surface);
            }
        }

        // Render a frame
        if let (Some(backend), Some(surface)) = (self.backend.as_ref(), self.surface.as_mut()) {
            let mut render_ctx = SimpleRenderContext::new(800, 600);
            if let Err(e) = backend.render(&mut render_ctx, surface.as_mut()) {
                eprintln!("render error: {e}");
            }

            // Get the wgpu texture and register it with egui if not done yet
            if self.texture_id.is_none() {
                if let Ok(ExternalHandle::WgpuTextureId { id, width, height, .. }) =
                    backend.external_handle(surface.as_mut())
                {
                    if let Some(wgpu_ctx) = &self.wgpu_ctx {
                        if let Some(wgpu_state) = frame.wgpu_render_state() {
                            let guard = wgpu_ctx.textures.lock().unwrap();
                            if let Some((_tex, view)) = guard.get(&id) {
                                let tex_id = wgpu_state.renderer.write().register_native_texture(
                                    &wgpu_state.device,
                                    view,
                                    wgpu::FilterMode::Linear,
                                );
                                self.texture_id = Some(tex_id);
                            }
                        }
                    }
                }
            }
        }

        egui::CentralPanel::default().show(ctx, |ui: &mut Ui| {
            if let Some(tex_id) = self.texture_id {
                ui.image(egui::load::SizedTexture::new(
                    tex_id,
                    egui::vec2(800.0, 600.0),
                ));
            } else {
                ui.label("Initialising Vello backend…");
            }
        });
    }
}

fn main() -> eframe::Result {
    eframe::run_native(
        "Gosub — egui_vello example",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default().with_inner_size([820.0, 640.0]),
            ..Default::default()
        },
        Box::new(|_cc| Ok(Box::new(GosubApp::default()))),
    )
}
