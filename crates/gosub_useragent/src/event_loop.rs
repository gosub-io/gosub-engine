use winit::event::WindowEvent;
use winit::event_loop::ActiveEventLoop;

use gosub_render_backend::{RenderBackend, SizeU32};
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

                tab.data
                    .mouse_move(backend, &mut self.renderer_data, position.x, position.y);
            }

            _ => {}
        }

        Ok(())
    }
}
