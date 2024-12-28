extern crate core;

use crate::callback::{FutureExecutor, TokioExecutor};
use crate::event_listeners::{EventListeners, Listeners};
use crate::timers::WebTimers;
use gosub_interface::config::HasWebComponents;
use gosub_interface::input::InputEvent;
use gosub_interface::instance::Handles;
use std::thread;
use tokio::runtime::{Handle, Runtime};
use tokio::sync::mpsc::{Receiver, Sender};
use tokio::task::LocalSet;

mod callback;
mod event_listeners;
pub mod poll_guard;
#[allow(dead_code)]
mod timers;

/// The web event loop, this will be the main event loop for a JS or Lua runtime, it is directly tied to an instance's EventLoop
#[allow(unused)]
pub struct WebEventLoop<C: HasWebComponents, E: FutureExecutor = TokioExecutor> {
    listeners: EventListeners<E>,
    rt: Handle,
    handles: Handles<C>,
    rx: Receiver<WebEventLoopMessage>,
    irx: Receiver<LocalEventLoopMessage<E>>,
    itx: Sender<LocalEventLoopMessage<E>>,
    timers: WebTimers,
}

/// Handle to the event loop, this can be used to spawn tasks or send messages to the event loop
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

impl<C: HasWebComponents> WebEventLoop<C> {
    /// Create a new WebEventLoop on a new thead, returning the handle to the event loop
    pub fn new_on_thread(handles: Handles<C>) -> WebEventLoopHandle {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        let handle = rt.handle().clone();

        let (tx, rx) = tokio::sync::mpsc::channel(100);

        thread::spawn(|| {
            let (itx, irx) = tokio::sync::mpsc::channel(100);
            let mut el = WebEventLoop {
                listeners: EventListeners::default(),
                handles,
                rt: rt.handle().clone(),
                irx,
                itx,
                rx,
                timers: WebTimers::new(),
            };

            el.run(rt, TokioExecutor);
        });

        WebEventLoopHandle { rt: handle, tx }
    }
}

impl<C: HasWebComponents, E: FutureExecutor> WebEventLoop<C, E> {
    pub fn run(&mut self, rt: Runtime, mut e: E) {
        let set = LocalSet::new();

        set.block_on(&rt, async {
            loop {
                tokio::select! {
                    val = self.rx.recv() => {
                        let Some(msg) = val else {
                            break;
                        };
                        self.handle_message(msg, &mut e);
                    }

                    val = self.irx.recv() => {
                        let Some(msg) = val else {
                            break;
                        };
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
