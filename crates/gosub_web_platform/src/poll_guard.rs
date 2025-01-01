use core::task;
use pin_project::pin_project;
use std::future::Future;
use std::pin::Pin;

pub trait PollCallback {
    fn poll(&mut self);
}

#[pin_project]
pub struct PollGuard<T: Future<Output = ()>, C: PollCallback> {
    cb: C,
    #[pin]
    fut: T,
}

impl<T: Future<Output = ()>, C: PollCallback> Future for PollGuard<T, C> {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut task::Context<'_>) -> task::Poll<()> {
        let this = self.project();
        this.cb.poll();

        this.fut.poll(cx)
    }
}

impl<F: Future<Output = ()>, C: PollCallback> PollGuard<F, C> {
    pub fn new(fut: F, cb: C) -> Self {
        Self { cb, fut }
    }
}
