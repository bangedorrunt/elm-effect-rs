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

/// The process-wide default runtime. Built on first use so a program that
/// never spawns a fiber starts no worker threads, and shared so N
/// [`crate::Runtime`] values do not mean N four-thread pools.
static SHARED: std::sync::OnceLock<tokio::runtime::Runtime> = std::sync::OnceLock::new();

/// The production executor: the shared Tokio multi-thread runtime, or one
/// built with explicit knobs.
pub struct TokioExecutor {
    runtime: ExecutorRuntime,
    max_blocking_threads: usize,
}

enum ExecutorRuntime {
    Shared(&'static tokio::runtime::Runtime),
    Owned(tokio::runtime::Runtime),
}

impl TokioExecutor {
    /// Defaults: `worker_threads = 4`, `max_blocking_threads = 4096`, on the
    /// shared runtime every default executor uses.
    pub fn new() -> Self {
        let runtime = SHARED.get_or_init(|| build(4, 4096));
        Self {
            runtime: ExecutorRuntime::Shared(runtime),
            max_blocking_threads: 4096,
        }
    }

    /// Build a private runtime with explicit knobs. `max_blocking_threads`
    /// caps concurrently running blocking threads; Tokio's blocking queue
    /// itself is unbounded, so this is not a backpressure bound on how many
    /// fibers may be pending.
    pub fn with_options(worker_threads: usize, max_blocking_threads: usize) -> Self {
        Self {
            runtime: ExecutorRuntime::Owned(build(worker_threads, max_blocking_threads)),
            max_blocking_threads,
        }
    }

    /// The configured cap on concurrently running blocking (fiber) threads.
    pub fn max_blocking_threads(&self) -> usize {
        self.max_blocking_threads
    }

    fn runtime(&self) -> &tokio::runtime::Runtime {
        match &self.runtime {
            ExecutorRuntime::Shared(runtime) => runtime,
            ExecutorRuntime::Owned(runtime) => runtime,
        }
    }
}

fn build(worker_threads: usize, max_blocking_threads: usize) -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(worker_threads)
        .max_blocking_threads(max_blocking_threads)
        .thread_name("elm-effect-fiber")
        .enable_all()
        .build()
        .expect("fiber runtime builds")
}

impl Default for TokioExecutor {
    fn default() -> Self {
        Self::new()
    }
}

impl Executor for TokioExecutor {
    fn spawn_boxed(&self, fut: BoxFuture<'static, ()>) {
        self.runtime().spawn(fut);
    }

    fn spawn_blocking(&self, f: Box<dyn FnOnce() + Send + 'static>) {
        self.runtime().spawn_blocking(f);
    }
}
