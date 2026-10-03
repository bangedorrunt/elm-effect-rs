//! Test scaffolding for elm-effect (`@effect/vitest` substitute).
//!
//! [`DeterministicExecutor`] runs one ready task at a time in registration
//! order on the calling thread: no background threads, no wall-clock time.
//! Timers complete only when the test clock says so (TestClock bead, Phase 3);
//! `tokio::time::pause` is not the spec and never runs here. Tasks that stay
//! pending (timers without a clock advance) are left queued: [`run_until_idle`]
//! stops when a full round makes no progress.
//!
//! [`run_until_idle`]: DeterministicExecutor::run_until_idle

use std::collections::VecDeque;
use std::sync::Mutex;
use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};

use elm_effect::{BoxFuture, Executor};

/// Single-threaded, registration-order executor for tests.
pub struct DeterministicExecutor {
    async_tasks: Mutex<VecDeque<BoxFuture<'static, ()>>>,
    blocking_tasks: Mutex<VecDeque<Box<dyn FnOnce() + Send + 'static>>>,
}

impl DeterministicExecutor {
    /// An empty executor. Tasks run only via [`run_until_idle`](Self::run_until_idle).
    pub fn new() -> Self {
        Self {
            async_tasks: Mutex::new(VecDeque::new()),
            blocking_tasks: Mutex::new(VecDeque::new()),
        }
    }

    /// Run queued work until a full round over both queues makes no progress.
    /// Returns the number of tasks completed. Pending-forever tasks (timers
    /// without a clock) stay queued; see [`pending`](Self::pending).
    pub fn run_until_idle(&self) -> usize {
        let mut completed = 0;
        loop {
            let mut progressed = false;
            if let Some(task) = self.blocking_tasks.lock().unwrap().pop_front() {
                task();
                completed += 1;
                progressed = true;
            }
            let mut tasks = self.async_tasks.lock().unwrap();
            let round = tasks.len();
            for _ in 0..round {
                let Some(mut task) = tasks.pop_front() else {
                    break;
                };
                let waker = noop_waker();
                let mut cx = Context::from_waker(&waker);
                match task.as_mut().poll(&mut cx) {
                    Poll::Ready(()) => {
                        completed += 1;
                        progressed = true;
                    }
                    Poll::Pending => tasks.push_back(task),
                }
            }
            drop(tasks);
            if !progressed {
                return completed;
            }
        }
    }

    /// Tasks still queued (never ready without a clock advance).
    pub fn pending(&self) -> usize {
        self.async_tasks.lock().unwrap().len() + self.blocking_tasks.lock().unwrap().len()
    }
}

impl Default for DeterministicExecutor {
    fn default() -> Self {
        Self::new()
    }
}

impl Executor for DeterministicExecutor {
    fn spawn_boxed(&self, fut: BoxFuture<'static, ()>) {
        self.async_tasks.lock().unwrap().push_back(fut);
    }

    fn spawn_blocking(&self, f: Box<dyn FnOnce() + Send + 'static>) {
        self.blocking_tasks.lock().unwrap().push_back(f);
    }
}

fn noop_waker() -> Waker {
    const VTABLE: RawWakerVTable = RawWakerVTable::new(clone, noop, noop, noop);
    unsafe fn clone(_: *const ()) -> RawWaker {
        RawWaker::new(std::ptr::null(), &VTABLE)
    }
    unsafe fn noop(_: *const ()) {}
    // SAFETY: the waker never dereferences its null data pointer.
    unsafe { Waker::from_raw(RawWaker::new(std::ptr::null(), &VTABLE)) }
}

/// A ready-made world for clock tests: a fiber [`Runtime`](elm_effect::Runtime)
/// plus a [`TestClock`](elm_effect::TestClock) that only `adjust` advances.
/// The `@effect/vitest` substitute: real threads, virtual time.
pub struct Fixture {
    pub rt: elm_effect::Runtime,
    pub clock: std::sync::Arc<elm_effect::TestClock>,
}

impl Fixture {
    pub fn new() -> Self {
        Self {
            rt: elm_effect::Runtime::new(),
            clock: std::sync::Arc::new(elm_effect::TestClock::new()),
        }
    }
}

impl Default for Fixture {
    fn default() -> Self {
        Self::new()
    }
}

/// A closed [`Layer`](elm_effect::Layer) serving this fixture's clock as the
/// [`Clock`](elm_effect::Clock) service.
pub fn clock_layer(
    clock: &std::sync::Arc<elm_effect::TestClock>,
) -> elm_effect::Layer<elm_effect::Clock, std::convert::Infallible, ()> {
    let shared: std::sync::Arc<dyn elm_effect::ClockImpl> = clock.clone();
    elm_effect::Layer::from_value(shared)
}

/// Declare a clock test with a runtime (`$rt`) and a virtual clock
/// (`$clock`) in scope. The legal sequence stays visible: fork sleepers,
/// `adjust` the clock, join the fibers.
#[macro_export]
macro_rules! effect_test {
    (fn $name:ident ($rt:ident, $clock:ident) $body:block) => {
        #[test]
        fn $name() {
            let __fixture = $crate::Fixture::new();
            let $rt = __fixture.rt;
            let $clock = __fixture.clock;
            $body
        }
    };
}
