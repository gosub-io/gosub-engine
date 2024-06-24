use winit::event::{MouseScrollDelta, WindowEvent};
use winit::event_loop::ActiveEventLoop;

use gosub_render_backend::{Point, RenderBackend, SizeU32, FP};
use gosub_renderer::draw::SceneDrawer;
use gosub_shared::types::Result;

use crate::window::{Window, WindowState};

impl<D: SceneDrawer<B>, B: RenderBackend> Window<'_, D, B> {
    pub fn event(
        &mut self,
        el: &ActiveEventLoop,
        backend: &mut B,
        event: WindowEvent,
    ) -> Result<()> {
        let WindowState::Active {
            surface: active_window_data,
        } = &mut self.state
        else {
            return Ok(());
        };

        let window = &self.window;

        match event {
            WindowEvent::CloseRequested => {
                el.exit();
            }
            WindowEvent::Resized(size) => {
                backend.resize_window(
                    &mut self.renderer_data,
                    active_window_data,
                    SizeU32::new(size.width, size.height),
                )?;
                window.request_redraw();
            }
            WindowEvent::RedrawRequested => {
                let size = window.inner_size();

                let size = SizeU32::new(size.width, size.height);

                let Some(tab) = self.tabs.get_current_tab() else {
                    return Ok(());
                };

                tab.data.draw(backend, &mut self.renderer_data, size);

                backend.render(&mut self.renderer_data, active_window_data)?;
            }

            WindowEvent::CursorMoved { position, .. } => {
                let Some(tab) = self.tabs.get_current_tab() else {
                    return Ok(());
                };

                if tab.data.mouse_move(
                    backend,
                    &mut self.renderer_data,
                    position.x as FP,
                    position.y as FP,
                ) {
                    self.window.request_redraw();
                }
            }

            WindowEvent::MouseWheel { delta, .. } => {
                let Some(tab) = self.tabs.get_current_tab() else {
                    return Ok(());
                };

                let delta = match delta {
                    MouseScrollDelta::PixelDelta(delta) => (delta.x as f32, delta.y as f32),
                    MouseScrollDelta::LineDelta(x, y) => (x * 12.0, y * 4.0),
                };

                let delta = Point::new(delta.0 as FP, delta.1 as FP);

                tab.data.scroll(delta);

                self.window.request_redraw();
            }

            _ => {}
        }

        Ok(())
    }
}
