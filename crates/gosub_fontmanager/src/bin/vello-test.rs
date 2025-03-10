use gosub_interface::font::FontStyle;
use std::num::NonZeroUsize;
use std::sync::Arc;
use vello::kurbo::{Affine, Circle, Ellipse, Line, RoundedRect, Stroke};
use vello::peniko::color;
use vello::util::{DeviceHandle, RenderContext, RenderSurface};
use vello::{wgpu, AaConfig, RenderParams, Renderer, RendererOptions, Scene};
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

const AA_CONFIGS: [AaConfig; 3] = [AaConfig::Area, AaConfig::Msaa8, AaConfig::Msaa16];

fn main() {
    colog::init();

    let event_loop = EventLoop::new().unwrap();
    event_loop.set_control_flow(ControlFlow::Poll);
    // event_loop.set_control_flow(ControlFlow::Wait);

    let mut app = App::new();
    let _ = event_loop.run_app(&mut app);
}

struct App<'s> {
    render_ctx: RenderContext,
    renderer: Option<Renderer>,
    surface: Option<RenderSurface<'s>>, // Surface must be before window for safety during cleanup
    window: Option<Arc<Window>>,
}

impl App<'_> {
    fn new() -> Self {
        App {
            window: None,
            render_ctx: RenderContext::new(),
            renderer: None,
            surface: None,
        }
    }
}

impl ApplicationHandler for App<'_> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let fontmanager = gosub_fontmanager::FontManager::new();
        let _ = fontmanager.find(&["Arial"], FontStyle::Normal).unwrap();

        let mut attribs = Window::default_attributes();
        attribs.title = "Vello Font Test".to_string();
        let window = Arc::new(event_loop.create_window(attribs).unwrap());

        let size = window.inner_size();
        let surface_future =
            self.render_ctx
                .create_surface(window.clone(), size.width, size.height, wgpu::PresentMode::AutoVsync);
        let surface = pollster::block_on(surface_future).expect("Failed to create surface");

        let dev_handle = &self.render_ctx.devices[surface.dev_id];

        let renderer = Renderer::new(
            &dev_handle.device,
            RendererOptions {
                surface_format: Some(surface.format),
                use_cpu: false,
                antialiasing_support: AA_CONFIGS.iter().copied().collect(),
                num_init_threads: NonZeroUsize::new(0),
            },
        );

        // let size = window.inner_size();
        // let config = wgpu::SurfaceConfiguration {
        //     usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        //     format: surface.get_supported_formats(&adapter)[0],
        //     width: size.width,
        //     height: size.height,
        //     present_mode: wgpu::PresentMode::Fifo,
        //     alpha_mode: wgpu::CompositeAlphaMode::Auto,
        //     view_formats: vec![],
        // };
        // surface.configure(&device, &config);

        // STEP 2: Create a scene

        // let mut scene = Scene::default();
        // scene.append(&Draw::Fill(Fill::new(
        //     FillStyle::default(),
        //     Transform::identity(),
        //     Rect::new(100.0, 100.0, 300.0, 300.0).into_path(),
        //     None,
        // )));

        self.window = Some(window);
        self.surface = Some(surface);
        self.renderer = Some(renderer.unwrap());
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                event_loop.exit();
            }
            WindowEvent::Resized(size) => {
                self.render_ctx
                    .resize_surface(self.surface.as_mut().unwrap(), size.width, size.height);
            }
            WindowEvent::RedrawRequested => {
                let surface = self.surface.as_ref().unwrap();

                let dev_id = surface.dev_id;
                let DeviceHandle { device, queue, .. } = &self.render_ctx.devices[dev_id];

                let width = surface.config.width;
                let height = surface.config.height;

                let surface_texture = surface
                    .surface
                    .get_current_texture()
                    .expect("Failed to get current texture");
                let render_params = RenderParams {
                    base_color: color::palette::css::YELLOW_GREEN,
                    width,
                    height,
                    antialiasing_method: AaConfig::Area,
                };

                let mut scene = Scene::new();
                // Draw an outlined rectangle
                let stroke = Stroke::new(6.0);
                let rect = RoundedRect::new(10.0, 10.0, 240.0, 240.0, 20.0);
                let rect_stroke_color = color::palette::css::YELLOW_GREEN;
                scene.stroke(&stroke, Affine::IDENTITY, rect_stroke_color, None, &rect);

                // Draw a filled circle
                let circle = Circle::new((420.0, 200.0), 120.0);
                let circle_fill_color = color::palette::css::REBECCA_PURPLE;
                scene.fill(
                    vello::peniko::Fill::NonZero,
                    Affine::IDENTITY,
                    circle_fill_color,
                    None,
                    &circle,
                );

                // Draw a filled ellipse
                let ellipse = Ellipse::new((250.0, 420.0), (100.0, 160.0), -90.0);
                let ellipse_fill_color = color::palette::css::BLUE_VIOLET;
                scene.fill(
                    vello::peniko::Fill::NonZero,
                    Affine::IDENTITY,
                    ellipse_fill_color,
                    None,
                    &ellipse,
                );

                // Draw a straight line
                let line = Line::new((260.0, 20.0), (620.0, 100.0));
                let line_stroke_color = color::palette::css::FIREBRICK;
                scene.stroke(&stroke, Affine::IDENTITY, line_stroke_color, None, &line);

                let _ = self.renderer.as_mut().unwrap().render_to_surface(
                    device,
                    queue,
                    &scene,
                    &surface_texture,
                    &render_params,
                );

                surface_texture.present();
            }
            _ => (),
        }
    }
}
