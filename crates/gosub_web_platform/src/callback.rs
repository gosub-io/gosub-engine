use crate::poll_guard::{PollCallback, PollGuard};
use std::future::Future;
use tokio::task;

struct TestPollGuard;

impl PollCallback for TestPollGuard {
    fn poll(&mut self) {
        //TODO: execute the js microtasks
    }
}

pub trait FutureExecutor {
    fn execute<T: Future<Output = ()> + 'static>(&mut self, future: T);
}

pub struct Callback<T: FutureExecutor, D = ()> {
    #[allow(clippy::type_complexity)]
    spawner: Box<dyn FnMut(&mut T, D)>,
}

impl<T: FutureExecutor, D> Callback<T, D> {
    pub fn new(spawner: impl FnMut(&mut T, D) + 'static) -> Self {
        Self {
            spawner: Box::new(spawner),
        }
    }

    pub fn execute(&mut self, executor: &mut T, data: D) {
        (self.spawner)(executor, data);
    }
}

impl<T: FutureExecutor> Callback<T> {
    pub fn exec(&mut self, executor: &mut T) {
        self.execute(executor, ());
    }
}

#[derive(Debug, Default)]
pub struct TokioExecutor;

impl FutureExecutor for TokioExecutor {
    fn execute<T: Future<Output = ()> + 'static>(&mut self, future: T) {
        let guard = PollGuard::new(future, TestPollGuard);

        task::spawn_local(guard);
    }
}
