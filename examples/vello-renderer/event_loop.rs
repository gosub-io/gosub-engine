use crate::window::{Window, WindowState};
use gosub_instance::{DebugEvent, InstanceMessage};
use gosub_interface::config::ModuleConfiguration;
use gosub_interface::draw::TreeDrawer;
use gosub_interface::input::InputEvent;
use gosub_interface::render_backend::{Point, RenderBackend, SizeU32, FP};
use gosub_shared::types::Result;
use winit::event::{ElementState, MouseScrollDelta, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::{KeyCode, ModifiersState, PhysicalKey};

impl<C: ModuleConfiguration> Window<'_, C> {
    pub fn event(&mut self, el: &ActiveEventLoop, backend: &mut C::RenderBackend, event: WindowEvent) -> Result<()> {
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

                tab.tx.blocking_send(InstanceMessage::Redraw(size))?;
            }

            WindowEvent::CursorMoved { position, .. } => {
                let Some(tab) = self.tabs.get_current_tab() else {
                    return Ok(());
                };

                tab.tx
                    .blocking_send(InstanceMessage::Input(InputEvent::MouseMove(Point::new(
                        position.x as FP,
                        position.y as FP,
                    ))))?;
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

                tab.tx
                    .blocking_send(InstanceMessage::Input(InputEvent::MouseScroll(delta)))?;
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
                            tab.tx.blocking_send(InstanceMessage::Debug(DebugEvent::Toggle))?;
                            self.window.request_redraw();
                        }
                        KeyCode::KeyC => {
                            tab.tx.blocking_send(InstanceMessage::Debug(DebugEvent::ClearBuffers))?;
                            self.window.request_redraw();
                        }
                        KeyCode::F5 => {
                            tab.tx.blocking_send(InstanceMessage::Reload)?;
                        }
                        KeyCode::ArrowRight => {
                            if self.mods.state().contains(ModifiersState::CONTROL) {
                                self.tabs.next_tab();
                                self.window.request_redraw();
                            }
                        }
                        KeyCode::ArrowLeft => {
                            if self.mods.state().contains(ModifiersState::CONTROL) {
                                self.tabs.previous_tab();
                                self.window.request_redraw();
                            }
                        }
                        KeyCode::Digit0 => {
                            if self.mods.state().contains(ModifiersState::CONTROL) {
                                self.tabs.activate_idx(0);
                                self.window.request_redraw();
                            }
                        }
                        KeyCode::Digit1 => {
                            if self.mods.state().contains(ModifiersState::CONTROL) {
                                self.tabs.activate_idx(1);
                                self.window.request_redraw();
                            }
                        }
                        KeyCode::Digit2 => {
                            if self.mods.state().contains(ModifiersState::CONTROL) {
                                self.tabs.activate_idx(2);
                                self.window.request_redraw();
                            }
                        }
                        KeyCode::Digit3 => {
                            if self.mods.state().contains(ModifiersState::CONTROL) {
                                self.tabs.activate_idx(3);
                                self.window.request_redraw();
                            }
                        }
                        KeyCode::Digit4 => {
                            if self.mods.state().contains(ModifiersState::CONTROL) {
                                self.tabs.activate_idx(4);
                                self.window.request_redraw();
                            }
                        }
                        KeyCode::Digit5 => {
                            if self.mods.state().contains(ModifiersState::CONTROL) {
                                self.tabs.activate_idx(5);
                                self.window.request_redraw();
                            }
                        }
                        KeyCode::Digit6 => {
                            if self.mods.state().contains(ModifiersState::CONTROL) {
                                self.tabs.activate_idx(6);
                                self.window.request_redraw();
                            }
                        }
                        KeyCode::Digit7 => {
                            if self.mods.state().contains(ModifiersState::CONTROL) {
                                self.tabs.activate_idx(7);
                                self.window.request_redraw();
                            }
                        }
                        KeyCode::Digit8 => {
                            if self.mods.state().contains(ModifiersState::CONTROL) {
                                self.tabs.activate_idx(8);
                                self.window.request_redraw();
                            }
                        }
                        KeyCode::Digit9 => {
                            if self.mods.state().contains(ModifiersState::CONTROL) {
                                self.tabs.activate_idx(9);
                                self.window.request_redraw();
                            }
                        }

                        // KeyCode::F6 => self.el.open_tab(Url::parse("https://news.ycombinator.com")?),
                        // KeyCode::F7 => self.el.open_tab(Url::parse("https://archlinux.org")?),
                        // KeyCode::F8 => self.el.open_tab(Url::parse("file://resources/test.html")?),
                        _ => {}
                    }
                }
            }

            WindowEvent::ModifiersChanged(mods) => {
                self.mods = mods;
            }

            _ => {}
        }

        Ok(())
    }
}
