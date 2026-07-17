extern crate core;

use crate::callback::{FutureExecutor, TokioExecutor};
use crate::event_listeners::{EventListeners, Listeners};
use crate::timers::WebTimers;
use gosub_interface::input::InputEvent;
use gosub_shared::types::Result;
use std::thread;
use tokio::runtime::{Handle, Runtime};
use tokio::sync::mpsc::{Receiver, Sender};
use tokio::task::LocalSet;

mod callback;
mod event_listeners;
pub mod poll_guard;
#[allow(dead_code)]
mod timers;

/// The web event loop for a JS or Lua runtime. Previously generic over `HasWebComponents`;
/// the rendering/chrome handles now live outside this crate.
#[allow(unused)]
pub struct WebEventLoop<E: FutureExecutor = TokioExecutor> {
    listeners: EventListeners<E>,
    rt: Handle,
    rx: Receiver<WebEventLoopMessage>,
    irx: Receiver<LocalEventLoopMessage<E>>,
    itx: Sender<LocalEventLoopMessage<E>>,
    timers: WebTimers,
}

/// Handle to the event loop - use to spawn tasks or send messages.
pub struct WebEventLoopHandle {
    pub rt: Handle,
    pub tx: Sender<WebEventLoopMessage>,
}

pub enum WebEventLoopMessage {
    InputEvent(InputEvent),
    Close,
}

pub enum LocalEventLoopMessage<E: FutureExecutor> {
    AddListener(Listeners<E>),
}

impl WebEventLoop {
    pub fn new_on_thread() -> Result<WebEventLoopHandle> {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
        let handle = rt.handle().clone();
        let (tx, rx) = tokio::sync::mpsc::channel(100);

        thread::spawn(|| {
            let (itx, irx) = tokio::sync::mpsc::channel(100);
            let mut el = WebEventLoop {
                listeners: EventListeners::default(),
                rt: rt.handle().clone(),
                irx,
                itx,
                rx,
                timers: WebTimers::new(),
            };
            el.run(rt, TokioExecutor);
        });

        Ok(WebEventLoopHandle { rt: handle, tx })
    }
}

impl<E: FutureExecutor> WebEventLoop<E> {
    pub fn run(&mut self, rt: Runtime, mut e: E) {
        let set = LocalSet::new();

        set.block_on(&rt, async {
            loop {
                tokio::select! {
                    val = self.rx.recv() => {
                        let Some(msg) = val else { break; };
                        self.handle_message(msg, &mut e);
                    }
                    val = self.irx.recv() => {
                        let Some(msg) = val else { break; };
                        self.handle_local_message(msg);
                    }
                }
            }
        });
    }

    fn handle_message(&mut self, msg: WebEventLoopMessage, exec: &mut E) {
        match msg {
            WebEventLoopMessage::InputEvent(e) => {
                self.listeners.handle_input_event(e, exec);
            }
            WebEventLoopMessage::Close => {
                self.rx.close();
            }
        }
    }

    fn handle_local_message(&mut self, msg: LocalEventLoopMessage<E>) {
        match msg {
            LocalEventLoopMessage::AddListener(listener) => {
                self.listeners.add_listener(listener);
            }
        }
    }
}
