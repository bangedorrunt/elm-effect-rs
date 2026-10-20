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
use std::sync::Arc;
use std::sync::Mutex;
use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};

use elm_effect::{BoxFuture, Executor};

/// Single-threaded, registration-order executor for tests. Clones
/// share one queue pair (like [`TestClock`]), so the handle kept beside
/// a [`elm_effect::Runtime`] built with it drains the same work the
/// runtime enqueued.
#[derive(Clone, Default)]
pub struct DeterministicExecutor {
    inner: std::sync::Arc<ExecutorInner>,
}

#[derive(Default)]
struct ExecutorInner {
    async_tasks: Mutex<VecDeque<BoxFuture<'static, ()>>>,
    blocking_tasks: Mutex<VecDeque<Box<dyn FnOnce() + Send + 'static>>>,
}

impl DeterministicExecutor {
    /// An empty executor. Tasks run only via [`run_until_idle`](Self::run_until_idle).
    pub fn new() -> Self {
        Self::default()
    }

    /// Run queued work until a full round over both queues makes no progress.
    /// Returns the number of tasks completed. Pending-forever tasks (timers
    /// without a clock) stay queued; see [`pending`](Self::pending).
    pub fn run_until_idle(&self) -> usize {
        let mut completed = 0;
        loop {
            let mut progressed = false;
            if let Some(task) = self.inner.blocking_tasks.lock().unwrap().pop_front() {
                task();
                completed += 1;
                progressed = true;
            }
            let mut tasks = self.inner.async_tasks.lock().unwrap();
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
        self.inner.async_tasks.lock().unwrap().len()
            + self.inner.blocking_tasks.lock().unwrap().len()
    }
}

impl Executor for DeterministicExecutor {
    fn spawn_boxed(&self, fut: BoxFuture<'static, ()>) {
        self.inner.async_tasks.lock().unwrap().push_back(fut);
    }

    fn spawn_blocking(&self, f: Box<dyn FnOnce() + Send + 'static>) {
        self.inner.blocking_tasks.lock().unwrap().push_back(f);
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

/// A seeded fake RNG: the `@effect/vitest` `TestRandom` substitute. Same seed,
/// same sequence, and the seeded stream matches [`Random`](elm_effect::Random)
/// draw-for-draw (same SplitMix64), so a seeded test replays what live code
/// would draw. [`feed`](Self::feed) queues scripted values consumed first, in
/// order; once the script runs out, draws fall back to the seeded stream.
///
/// Clones share one handle (like [`TestClock`], unlike `Random::clone` which
/// snapshots): forked fibers draw from the same script in draw order.
#[derive(Clone)]
pub struct TestRandom {
    shared: Arc<Mutex<TestRandomState>>,
}

struct TestRandomState {
    seed: u64,
    state: u64,
    seeded_draws: u64,
    script: VecDeque<f64>,
}

impl TestRandom {
    /// A fake RNG replaying `seed`. Same seed, same sequence.
    pub fn new(seed: u64) -> Self {
        Self {
            shared: Arc::new(Mutex::new(TestRandomState {
                seed,
                state: seed | 1,
                seeded_draws: 0,
                script: VecDeque::new(),
            })),
        }
    }

    /// Queue scripted values, consumed before the seeded stream, in order.
    /// Tests script one lane while the rest of the stream stays seeded.
    pub fn feed(&self, values: impl IntoIterator<Item = f64>) {
        self.shared.lock().unwrap().script.extend(values);
    }

    /// Next unit in `[0, 1)`: script head first, else the seeded stream.
    pub fn next_unit(&self) -> f64 {
        let mut guard = self.shared.lock().unwrap();
        if let Some(v) = guard.script.pop_front() {
            return v;
        }
        guard.seeded_draws += 1;
        next_splitmix(&mut guard.state)
    }

    /// A live [`Random`](elm_effect::Random) continuing this seed's stream
    /// past the draws already taken from it. Scripted values are a
    /// handle-side overlay and do not cross into the service snapshot.
    pub fn to_random(&self) -> elm_effect::Random {
        let (seed, draws) = {
            let guard = self.shared.lock().unwrap();
            (guard.seed, guard.seeded_draws)
        };
        let rng = elm_effect::Random::with_seed(seed);
        for _ in 0..draws {
            rng.next_unit();
        }
        rng
    }
}

/// One SplitMix64 step, matching `Random::next_unit` draw-for-draw.
fn next_splitmix(state: &mut u64) -> f64 {
    *state = state.wrapping_add(0x9E3779B97F4A7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
    z ^= z >> 31;
    f64::from_bits((z >> 11) | 0x3FF0000000000000u64) - 1.0
}

/// A closed [`Layer`](elm_effect::Layer) serving this fake's seeded stream as
/// the [`Random`](elm_effect::Random) service. The `clock_layer` equivalent
/// for randomness: code written against `RandomTag` runs sealed in tests.
pub fn random_layer(
    random: &TestRandom,
) -> elm_effect::Layer<elm_effect::RandomTag, std::convert::Infallible, ()> {
    elm_effect::Layer::from_value(random.to_random())
}

/// The test world in one bundle: virtual time plus fake randomness. Tests
/// hold this instead of touching the wall clock or a live RNG.
pub struct TestContext {
    pub clock: Arc<elm_effect::TestClock>,
    pub random: TestRandom,
}

impl TestContext {
    /// Virtual clock at zero plus a fake RNG replaying `seed`.
    pub fn new(seed: u64) -> Self {
        Self {
            clock: Arc::new(elm_effect::TestClock::new()),
            random: TestRandom::new(seed),
        }
    }

    /// This world's clock as the [`Clock`](elm_effect::Clock) service.
    pub fn clock_layer(
        &self,
    ) -> elm_effect::Layer<elm_effect::Clock, std::convert::Infallible, ()> {
        clock_layer(&self.clock)
    }

    /// This world's RNG as the [`Random`](elm_effect::Random) service.
    pub fn random_layer(
        &self,
    ) -> elm_effect::Layer<elm_effect::RandomTag, std::convert::Infallible, ()> {
        random_layer(&self.random)
    }
}

/// The one explicit door back to live services. Tests default to
/// [`TestContext`]; reaching for this is visible in the diff, so no test
/// touches real time or randomness by accident.
pub struct TestLive;

impl TestLive {
    /// The wall clock as the [`Clock`](elm_effect::Clock) service, for the
    /// test that proves it needs real time. Never the default.
    pub fn system_clock_layer() -> elm_effect::Layer<elm_effect::Clock, std::convert::Infallible, ()>
    {
        let shared: Arc<dyn elm_effect::ClockImpl> = Arc::new(elm_effect::SystemClock);
        elm_effect::Layer::from_value(shared)
    }
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
