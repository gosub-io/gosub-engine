use crate::callback::{Callback, FutureExecutor};
use gosub_interface::input::{InputEvent, MouseButton};
use gosub_shared::geo::Point;
use std::fmt::Debug;

pub enum Listeners<E: FutureExecutor> {
    MouseDown(Callback<E, MouseButtonEvent>),
    MouseUp(Callback<E, MouseButtonEvent>),
    MouseMove(Callback<E, MouseMoveEvent>),
    MouseScroll(Callback<E, MouseScrollEvent>),
    KeyboardUp(Callback<E, KeyboardEvent>),
    KeyboardDown(Callback<E, KeyboardEvent>),
}

#[derive(Debug, Clone, Copy)]
pub struct MouseButtonEvent {
    pub button: MouseButton,
}

#[derive(Debug, Clone, Copy)]
pub struct KeyboardEvent {
    pub key: char,
}

#[derive(Debug, Clone, Copy)]
pub struct MouseMoveEvent {
    pub pos: Point,
}

#[derive(Debug, Clone, Copy)]
pub struct MouseScrollEvent {
    pub delta: Point,
}

pub struct EventListener<D, E: FutureExecutor> {
    listeners: Vec<Callback<E, D>>,
}

impl<D: Clone + Debug, E: FutureExecutor> EventListener<D, E> {
    pub fn handle_event(&mut self, event: D, e: &mut E) {
        for listener in self.listeners.iter_mut() {
            listener.execute(e, event.clone());
        }
    }
}

impl<D, E: FutureExecutor> Debug for EventListener<D, E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EventListener")
            .field("listeners", &self.listeners.len())
            .finish()
    }
}

impl<D, E: FutureExecutor> Default for EventListener<D, E> {
    fn default() -> Self {
        Self { listeners: Vec::new() }
    }
}

pub struct EventListeners<E: FutureExecutor> {
    mouse_up: EventListener<MouseButtonEvent, E>,
    mouse_down: EventListener<MouseButtonEvent, E>,
    mouse_move: EventListener<MouseMoveEvent, E>,
    mouse_scroll: EventListener<MouseScrollEvent, E>,
    keyboard_up: EventListener<KeyboardEvent, E>,
    keyboard_down: EventListener<KeyboardEvent, E>,
}

impl<E: FutureExecutor> EventListeners<E> {
    pub(crate) fn add_listener(&mut self, listener: Listeners<E>) {
        match listener {
            Listeners::MouseDown(callback) => self.mouse_down.listeners.push(callback),
            Listeners::MouseUp(callback) => self.mouse_up.listeners.push(callback),
            Listeners::MouseMove(callback) => self.mouse_move.listeners.push(callback),
            Listeners::MouseScroll(callback) => self.mouse_scroll.listeners.push(callback),
            Listeners::KeyboardUp(callback) => self.keyboard_up.listeners.push(callback),
            Listeners::KeyboardDown(callback) => self.keyboard_down.listeners.push(callback),
        }
    }

    pub(crate) fn handle_input_event(&mut self, event: InputEvent, e: &mut E) {
        match event {
            InputEvent::MouseDown(button) => {
                self.mouse_down.handle_event(MouseButtonEvent { button }, e);
            }
            InputEvent::MouseUp(button) => {
                self.mouse_up.handle_event(MouseButtonEvent { button }, e);
            }
            InputEvent::MouseMove(pos) => {
                self.mouse_move.handle_event(MouseMoveEvent { pos }, e);
            }
            InputEvent::MouseScroll(delta) => {
                self.mouse_scroll.handle_event(MouseScrollEvent { delta }, e);
            }
            InputEvent::KeyboardDown(key) => {
                self.keyboard_down.handle_event(KeyboardEvent { key }, e);
            }
            InputEvent::KeyboardUp(key) => {
                self.keyboard_up.handle_event(KeyboardEvent { key }, e);
            }
        }
    }
}

impl<E: FutureExecutor> Default for EventListeners<E> {
    fn default() -> Self {
        Self {
            mouse_up: EventListener::default(),
            mouse_down: EventListener::default(),
            mouse_move: EventListener::default(),
            mouse_scroll: EventListener::default(),
            keyboard_up: EventListener::default(),
            keyboard_down: EventListener::default(),
        }
    }
}

impl<E: FutureExecutor> Debug for EventListeners<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EventListeners")
            .field("mouse_down", &self.mouse_down)
            .field("mouse_up", &self.mouse_up)
            .field("mouse_move", &self.mouse_move)
            .field("mouse_scroll", &self.mouse_scroll)
            .field("keyboard_down", &self.keyboard_down)
            .field("keyboard_up", &self.keyboard_up)
            .finish()
    }
}
