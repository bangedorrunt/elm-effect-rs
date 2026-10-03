//! The [`Executor`] boundary: the only way work reaches threads.
//!
//! Production uses [`TokioExecutor`]; tests use the deterministic executor in
//! `elm-effect-test` (registration order, no background threads). Pool knobs
//! live on the executor constructor, never inside combinators.

use std::future::Future;
use std::pin::Pin;

/// A boxed one-shot future the executor drives to completion.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Spawns work without running it. Implementations never interpret effects.
pub trait Executor: Send + Sync + 'static {
    /// Drive `fut` to completion on the pool.
    fn spawn_boxed(&self, fut: BoxFuture<'static, ()>);
    /// Run `f` to completion on the blocking pool (fibers live here).
    fn spawn_blocking(&self, f: Box<dyn FnOnce() + Send + 'static>);
}

/// The production executor: one shared Tokio multi-thread runtime.
pub struct TokioExecutor {
    runtime: tokio::runtime::Runtime,
    queue_bound: usize,
}

impl TokioExecutor {
    /// Defaults: `worker_threads = 4`, `queue_bound = 4096`.
    pub fn new() -> Self {
        Self::with_options(4, 4096)
    }

    /// Override both pool knobs. `queue_bound` caps the blocking pool, which
    /// is where fiber bodies queue.
    pub fn with_options(worker_threads: usize, queue_bound: usize) -> Self {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(worker_threads)
            .max_blocking_threads(queue_bound)
            .thread_name("elm-effect-fiber")
            .enable_all()
            .build()
            .expect("fiber runtime builds");
        Self {
            runtime,
            queue_bound,
        }
    }

    /// The configured bound on queued blocking (fiber) work.
    pub fn queue_bound(&self) -> usize {
        self.queue_bound
    }
}

impl Default for TokioExecutor {
    fn default() -> Self {
        Self::new()
    }
}

impl Executor for TokioExecutor {
    fn spawn_boxed(&self, fut: BoxFuture<'static, ()>) {
        self.runtime.spawn(fut);
    }

    fn spawn_blocking(&self, f: Box<dyn FnOnce() + Send + 'static>) {
        self.runtime.spawn_blocking(f);
    }
}
