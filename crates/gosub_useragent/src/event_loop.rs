use winit::event::{ElementState, MouseScrollDelta, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::{KeyCode, PhysicalKey};

use gosub_render_backend::{FP, Point, RenderBackend, SizeU32};
use gosub_render_backend::layout::{Layouter, LayoutTree};
use gosub_renderer::draw::SceneDrawer;
use gosub_shared::traits::css3::CssSystem;
use gosub_shared::traits::document::Document;
use gosub_shared::types::Result;

use crate::window::{Window, WindowState};

impl<
        'a,
        D: SceneDrawer<B, L, LT, Doc, C>,
        B: RenderBackend,
        L: Layouter,
        LT: LayoutTree<L>,
        Doc: Document<C>,
        C: CssSystem,
    > Window<'a, D, B, L, LT, Doc, C>
{
    pub fn event(&mut self, el: &ActiveEventLoop, backend: &mut B, event: WindowEvent) -> Result<()> {
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


                let w = window.clone();

                let redraw = tab.data.draw(backend, &mut self.renderer_data, size, move || {
                    w.request_redraw();
                });
                
                backend.render(&mut self.renderer_data, active_window_data)?;
                
                if redraw {
                    self.request_redraw();
                }
            }

            WindowEvent::CursorMoved { position, .. } => {
                let Some(tab) = self.tabs.get_current_tab() else {
                    return Ok(());
                };

                if tab.data.mouse_move(backend, position.x as FP, position.y as FP) {
                    self.window.request_redraw();
                }
            }

            WindowEvent::MouseWheel { delta, .. } => {
                let Some(tab) = self.tabs.get_current_tab() else {
                    return Ok(());
                };

                let delta = match delta {
                    MouseScrollDelta::PixelDelta(delta) => (delta.x as f32, delta.y as f32),
                    MouseScrollDelta::LineDelta(x, y) => (x * 4.0, y * 12.0),
                };

                let delta = Point::new(delta.0 as FP, delta.1 as FP);

                tab.data.scroll(delta);

                self.window.request_redraw();
            }

            WindowEvent::KeyboardInput { event, .. } => {
                if event.repeat || event.state == ElementState::Released {
                    return Ok(());
                }

                let Some(tab) = self.tabs.get_current_tab() else {
                    return Ok(());
                };

                if let PhysicalKey::Code(code) = event.physical_key {
                    match code {
                        KeyCode::KeyD => {
                            tab.data.toggle_debug();
                            self.window.request_redraw();
                        }
                        KeyCode::KeyC => {
                            tab.data.clear_buffers();
                            self.window.request_redraw();
                        }
                        _ => {}
                    }
                }
            }

            _ => {}
        }

        Ok(())
    }
}
