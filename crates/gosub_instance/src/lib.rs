use gosub_interface::chrome::ChromeHandle;
use gosub_interface::config::{HasTreeDrawer, ModuleConfiguration};
use gosub_interface::draw::TreeDrawer;
use gosub_interface::eventloop::EventLoopHandle;
use gosub_interface::input::InputEvent;
use gosub_interface::instance::{Handles, InstanceId};
use gosub_interface::layout::LayoutTree;
use gosub_interface::render_backend::{ImageBuffer, NodeDesc};
use gosub_net::http::fetcher::Fetcher;
use gosub_shared::geo::SizeU32;
use gosub_shared::types::Result;
use log::warn;
use std::sync::mpsc::Sender as SyncSender;
use std::sync::Arc;
use tokio::runtime::{Builder, Handle, Runtime};
use tokio::sync::mpsc::{Receiver, Sender};
use tokio::task;
use tokio::task::LocalSet;
use url::Url;

/// Represents a running instance of the engine. This can be a tab in a browser or a webview
pub struct EngineInstance<C: ModuleConfiguration> {
    pub title: String,
    pub url: Url,
    pub data: C::TreeDrawer,
    rx: Receiver<InstanceMessage>,
    irx: Receiver<InternalInstanceMessage<C>>,
    el: El<C>,
    id: InstanceId,
    handles: Handles<C>,
    #[allow(unused)]
    fetcher: Arc<Fetcher>,
    size: SizeU32,
}

impl<C: ModuleConfiguration> EngineInstance<C> {
    pub async fn new(
        url: Url,
        layouter: C::Layouter,
        id: InstanceId,
        handles: Handles<C>,
    ) -> Result<(Self, InstanceHandle)> {
        let (tx, rx) = tokio::sync::mpsc::channel(128);

        let instance = EngineInstance::with_chan(url.clone(), layouter, rx, id, handles).await?;

        let handle = InstanceHandle { tx };

        Ok((instance, handle))
    }

    pub async fn with_chan(
        url: Url,
        layouter: C::Layouter,
        rx: Receiver<InstanceMessage>,
        id: InstanceId,
        handles: Handles<C>,
    ) -> Result<Self> {
        let fetcher = Arc::new(Fetcher::new(url.clone()));
        let data = C::TreeDrawer::with_fetcher(url.clone(), fetcher.clone(), layouter, false).await?;

        let (itx, irx) = tokio::sync::mpsc::channel(128);

        Ok(EngineInstance {
            title: "Gosub".to_string(),
            url,
            data,
            rx,
            el: El(itx),
            irx,
            id,
            handles,
            fetcher,
            size: SizeU32::new(0, 0),
        })
    }

    /// Spawns a new `EngineInstance` on a new thread, returning the `InstanceHandle` to communicate with it
    pub fn new_on_thread(url: Url, layouter: C::Layouter, id: InstanceId, handles: Handles<C>) -> Result<InstanceHandle>
    where
        C::Layouter: Send + 'static,
    {
        let (tx, rx) = tokio::sync::mpsc::channel(128);

        std::thread::spawn(move || {
            let rt = Builder::new_current_thread().enable_all().build().unwrap();

            let mut instance = match rt.block_on(Self::with_chan(url, layouter, rx, id, handles)) {
                Ok(instance) => instance,
                Err(e) => {
                    eprintln!("Error: {:?}", e);
                    return;
                }
            };

            instance.run(&rt);
        });

        Ok(InstanceHandle { tx })
    }

    /// Runs the instance on the current thread with the given `Runtime`
    fn run(&mut self, rt: &Runtime) {
        let set = LocalSet::new();

        set.block_on(rt, async move {
            while let Some(message) = self.rx.recv().await {
                if let Err(e) = self.handle_message(message).await {
                    warn!("Error: {:?}", e);
                }
            }
        });
    }

    /// Handles a message sent to the instance
    async fn handle_message(&mut self, message: InstanceMessage) -> Result<()> {
        match message {
            InstanceMessage::Redraw(size) => {
                self.size = size;
                let scene = self.data.draw(size, &self.el);

                self.handles.chrome.draw_scene(scene, size, self.id);
            }

            InstanceMessage::Navigate(url) => {
                let el = self.el.clone();

                task::spawn_local(self.data.navigate(url, el));
            }

            InstanceMessage::Back => {
                //TODO
            }

            InstanceMessage::Forward => {
                //TODO
            }

            InstanceMessage::Reload => {
                let el = self.el.clone();

                task::spawn_local(self.data.reload(el));
            }

            InstanceMessage::Close => {
                self.rx.close();
                self.irx.close();
            }

            InstanceMessage::Debug(event) => {
                match event {
                    DebugEvent::SendNodes(sender) => {
                        self.data.send_nodes(sender);
                    }

                    DebugEvent::SelectElement(id) => {
                        self.data
                            .select_element(<C::LayoutTree as LayoutTree<C>>::NodeId::from(id));
                    }

                    DebugEvent::Info(id, sender) => {
                        self.data
                            .info(<C::LayoutTree as LayoutTree<C>>::NodeId::from(id), sender);
                    }

                    DebugEvent::Deselect => {
                        self.data.unselect_element();
                    }

                    DebugEvent::Toggle => {
                        self.data.toggle_debug();
                    }

                    DebugEvent::Enable => {
                        self.data.toggle_debug(); //TODO
                    }

                    DebugEvent::Disable => {
                        self.data.toggle_debug();
                    }

                    DebugEvent::ClearBuffers => {
                        self.data.clear_buffers();
                    }
                }
            }
            InstanceMessage::Input(event) => match event {
                InputEvent::MouseScroll(delta) => {
                    self.data.scroll(delta);
                    self.redraw();
                }
                InputEvent::MouseMove(point) => {
                    if self.data.mouse_move(point.x, point.y) {
                        self.redraw();
                    }
                }
                _ => {} //TODO: send all events to the WebEventLoop
            },
        }

        Ok(())
    }

    fn redraw(&mut self) {
        let scene = self.data.draw(self.size, &self.el);

        self.handles.chrome.draw_scene(scene, self.size, self.id);
    }
}

pub struct InstanceHandle {
    pub tx: Sender<InstanceMessage>,
}

pub enum InstanceMessage {
    /// Redraw the instance with the given size
    Redraw(SizeU32),

    /// Navigate to the given URL
    Navigate(Url),
    /// Navigate back in history
    Back,
    /// Navigate forward in history
    Forward,
    /// Reload the current page
    Reload,
    /// Close the instance
    Close,

    /// Input event (mouse, keyboard, etc.)
    Input(InputEvent),
    /// Debug event (send nodes, select element, etc.)
    Debug(DebugEvent),
}

#[derive(Clone)]
struct El<C: ModuleConfiguration>(Sender<InternalInstanceMessage<C>>);

impl<C: ModuleConfiguration> EventLoopHandle<C> for El<C> {
    fn redraw(&self) {
        self.send(InternalInstanceMessage::Redraw);
    }

    fn add_img_cache(&self, url: Url, buf: ImageBuffer<C::RenderBackend>, size: Option<SizeU32>) {
        self.send(InternalInstanceMessage::Image(url, buf, size));
    }

    fn reload_from(&self, rt: C::RenderTree) {
        self.send(InternalInstanceMessage::ReloadFrom(rt));
    }
}

impl<C: ModuleConfiguration> El<C> {
    fn send(&self, message: InternalInstanceMessage<C>) {
        let send = self.0.clone();

        if let Ok(handle) = Handle::try_current() {
            handle.spawn(async move {
                let _ = send.send(message).await;
            });
        } else {
            let _ = send.blocking_send(message);
        }
    }
}

pub enum InternalInstanceMessage<C: HasTreeDrawer> {
    /// Add an image to the cache
    Image(Url, ImageBuffer<C::RenderBackend>, Option<SizeU32>),
    /// Redraw the instance
    Redraw,
    /// Reload the instance from the given tree
    ReloadFrom(C::RenderTree),
}

pub enum DebugEvent {
    /// Send a NodeDescription of the root node to the given sender
    SendNodes(SyncSender<NodeDesc>),
    /// Visually select the element with the given ID
    SelectElement(u64),
    /// Send a NodeDescription of the element with the given ID to the given sender
    Info(u64, SyncSender<NodeDesc>),
    /// Deselect the currently selected element (visually)
    Deselect,
    /// Toggle the debug mode
    Toggle,
    /// Enable the debug mode
    Enable,
    /// Disable the debug mode
    Disable,
    /// Clear the debug buffers so the next draw will be a full redraw
    ClearBuffers,
}
