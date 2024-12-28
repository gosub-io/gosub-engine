use crate::callback::{Callback, TokioExecutor};
use slotmap::{DefaultKey, SlotMap};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;
use tokio::task;
use tokio::task::JoinHandle;

#[derive(Debug)]
pub struct WebTimers {
    inner: Rc<RefCell<WebTimersInner>>,
}

#[derive(Debug)]
pub struct WebTimersInner {
    timers: SlotMap<DefaultKey, Timer>,
}

impl WebTimers {
    pub fn new() -> Self {
        Self {
            inner: Rc::new(RefCell::new(WebTimersInner { timers: SlotMap::new() })),
        }
    }

    pub fn add_timer(&mut self, timer: Timer) -> TimerId {
        TimerId(self.inner.borrow_mut().timers.insert(timer))
    }

    /// Removes and cancels the timer with the given id.
    pub fn remove(&mut self, id: TimerId) {
        if let Some(timer) = self.inner.borrow_mut().timers.remove(id.0) {
            timer.handle.abort();
        }
    }

    pub fn set_timeout(&mut self, duration: Duration, mut callback: Callback<TokioExecutor>) {
        let inner = self.inner.clone();

        self.inner.borrow_mut().timers.insert_with_key(move |key| {
            let handle = task::spawn_local(async move {
                tokio::time::sleep(duration).await;

                callback.exec(&mut TokioExecutor);

                inner.borrow_mut().timers.remove(key);
            });

            Timer { handle }
        });
    }

    pub fn set_interval(&mut self, duration: Duration, mut callback: Callback<TokioExecutor>) {
        let handle = task::spawn_local(async move {
            let mut interval = tokio::time::interval(duration);

            interval.tick().await; // First tick is immediate

            loop {
                interval.tick().await;
                callback.exec(&mut TokioExecutor);
            }
        });

        let timer = Timer { handle };

        self.inner.borrow_mut().timers.insert(timer);
    }

    pub fn remove_all(&mut self) {
        for (_, timer) in self.inner.borrow_mut().timers.drain() {
            timer.handle.abort();
        }
    }
}

#[derive(Debug)]
pub struct Timer {
    handle: JoinHandle<()>,
}

#[derive(Debug)]
pub struct TimerId(DefaultKey);
