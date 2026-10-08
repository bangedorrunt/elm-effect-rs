//! Pull-based streams: each pull rebuilds its effect (effects are
//! one-shot, so recipes are factories, like layers and retries).
//!
//! An unbounded input stays a stream of bounded chunks: [`from_iter`]
//! pulls `chunk_size` items at a time and never materializes the rest, so
//! `take(5)` over an infinite iterator terminates. Transforms stay lazy;
//! only the `run_*` sinks interpret pulls, eagerly, like [`Runtime::run`].
//! Sinks exist only on closed streams (`R = ()`): provide services first.
//!
//! Sharing is [`Arc`] over [`Mutex`]-guarded factories, so only `Send` is
//! ever required: requirement tags (which are not `Sync`) stream fine.

use std::convert::Infallible;
use std::sync::{Arc, Mutex};

use crate::{Effect, Exit};

/// A shared pull factory: rebuilt effects, one call at a time.
type Pull<A, E, R> = Arc<Mutex<Box<dyn FnMut() -> Effect<Option<Vec<A>>, E, R> + Send>>>;

/// A pull-based stream. Pulls yield bounded chunks; `None` ends the stream.
/// Empty chunks are skipped, never terminal.
/// Shared fan-out state: an owned worker runtime plus the currently
/// registered batch children, so [`cancel`](Stream::cancel) can interrupt
/// and join them.
struct StreamExec {
    runtime: Mutex<Option<std::sync::Arc<crate::Runtime>>>,
    inflight: Mutex<
        std::collections::HashMap<u64, Vec<crate::FiberHandle<(), std::convert::Infallible>>>,
    >,
    next_batch: std::sync::atomic::AtomicU64,
}

impl StreamExec {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            runtime: Mutex::new(None),
            inflight: Mutex::new(std::collections::HashMap::new()),
            next_batch: std::sync::atomic::AtomicU64::new(0),
        })
    }

    /// The shared worker runtime, created on first fan-out.
    fn runtime(exec: &Arc<Self>) -> std::sync::Arc<crate::Runtime> {
        let mut guard = exec.runtime.lock().unwrap();
        if let Some(rt) = guard.as_ref() {
            return Arc::clone(rt);
        }
        let rt = Arc::new(crate::Runtime::new());
        *guard = Some(Arc::clone(&rt));
        rt
    }
}

pub struct Stream<A: Send + 'static, E: Send + 'static, R: crate::effect::Requirement> {
    pull: Pull<A, E, R>,
    exec: Arc<StreamExec>,
}

impl<A: Send + 'static, E: Send + 'static, R: crate::effect::Requirement> Clone
    for Stream<A, E, R>
{
    fn clone(&self) -> Self {
        Self {
            pull: Arc::clone(&self.pull),
            exec: Arc::clone(&self.exec),
        }
    }
}

/// Call a shared pull factory once.
fn call<A: Send + 'static, E: Send + 'static, R: crate::effect::Requirement>(
    pull: &Pull<A, E, R>,
) -> Effect<Option<Vec<A>>, E, R> {
    (pull.lock().unwrap())()
}

impl<A: Send + 'static, E: Send + 'static, R: crate::effect::Requirement> Stream<A, E, R> {
    /// Build from a pull factory. `None` ends the stream.
    pub fn from_pull(pull: impl FnMut() -> Effect<Option<Vec<A>>, E, R> + Send + 'static) -> Self {
        Self {
            pull: Arc::new(Mutex::new(Box::new(pull))),
            exec: StreamExec::new(),
        }
    }

    /// Build from a pull factory, sharing execution state (runtime, batch
    /// registry) with the source stream. Transforms use this; sources use
    /// [`from_pull`](Stream::from_pull), which starts fresh state.
    fn derive<B2, E2, R2>(
        exec: Arc<StreamExec>,
        pull: impl FnMut() -> Effect<Option<Vec<B2>>, E2, R2> + Send + 'static,
    ) -> Stream<B2, E2, R2>
    where
        B2: Send + 'static,
        E2: Send + 'static,
        R2: crate::effect::Requirement,
    {
        Stream {
            pull: Arc::new(Mutex::new(Box::new(pull) as Box<dyn FnMut() -> _ + Send>)),
            exec,
        }
    }

    /// An empty stream.
    pub fn empty() -> Self {
        Self::from_pull(|| Effect::succeed(None))
    }

    /// Map every element, lazily.
    pub fn map<B, F>(self, f: F) -> Stream<B, E, R>
    where
        B: Send + 'static,
        F: Fn(A) -> B + Send + Sync + 'static,
    {
        let (pull, f, exec) = (self.pull, Arc::new(f), Arc::clone(&self.exec));
        Self::derive(exec, move || {
            let f = Arc::clone(&f);
            call(&pull)
                .map(move |chunk| chunk.map(|items| items.into_iter().map(|a| f(a)).collect()))
        })
    }

    /// Keep elements satisfying the predicate, lazily.
    pub fn filter<F>(self, f: F) -> Self
    where
        F: Fn(&A) -> bool + Send + Sync + 'static,
    {
        let (pull, f, exec) = (self.pull, Arc::new(f), Arc::clone(&self.exec));
        Self::derive(exec, move || {
            let f = Arc::clone(&f);
            call(&pull)
                .map(move |chunk| chunk.map(|items| items.into_iter().filter(|a| f(a)).collect()))
        })
    }

    /// Take at most `n` elements, then end.
    pub fn take(self, n: u64) -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        let (pull, remaining, exec) = (
            self.pull,
            Arc::new(AtomicU64::new(n)),
            Arc::clone(&self.exec),
        );
        Self::derive(exec, move || {
            if remaining.load(Ordering::SeqCst) == 0 {
                return Effect::succeed(None);
            }
            let remaining = Arc::clone(&remaining);
            call(&pull).map(move |chunk| {
                chunk.and_then(|mut items| {
                    let left = remaining.load(Ordering::SeqCst);
                    let take = (left as usize).min(items.len());
                    remaining.fetch_sub(take as u64, Ordering::SeqCst);
                    items.truncate(take);
                    if items.is_empty() { None } else { Some(items) }
                })
            })
        })
    }

    /// Run `other` after `self` ends.
    pub fn concat(self, other: Self) -> Self {
        use std::sync::atomic::{AtomicBool, Ordering};
        let exec = Arc::clone(&self.exec);
        let (first, second, drained) = (
            self.pull,
            Arc::new(Mutex::new(other.pull)),
            Arc::new(AtomicBool::new(false)),
        );
        Self::derive(exec, move || {
            if drained.load(Ordering::SeqCst) {
                return call(&second.lock().unwrap());
            }
            let (drained, second) = (Arc::clone(&drained), Arc::clone(&second));
            call(&first).flat_map(move |chunk| match chunk {
                None => {
                    drained.store(true, Ordering::SeqCst);
                    call(&second.lock().unwrap())
                }
                Some(items) => Effect::succeed(Some(items)),
            })
        })
    }
}

/// Mirror of the pin's `Count.normalizeNonEmpty` (`internal/count.ts`):
/// `NaN` and non-positive values become 1, a positive value floors, and
/// a positive infinity saturates to `usize::MAX` exactly as `Math.floor`
/// preserves it in TypeScript.
pub fn normalize_non_empty(n: f64) -> usize {
    if n > 0.0 {
        n.floor() as usize
    } else {
        1
    }
}

/// The pin's `Channel.DefaultChunkSize` (`Channel.ts:458`).
pub const DEFAULT_CHUNK_SIZE: f64 = 4096.0;

impl Stream<i64, Infallible, ()> {
    /// `Stream.range` at the pin: inclusive integer range, chunked. `min` above
    /// `max` is empty; the chunk size normalizes like the pin's `Count`. Pulls
    /// yield chunks of `normalize(chunk_size)` until the range is exhausted,
    /// then end (`None`), never a zero-length chunk.
    pub fn range(min: i64, max: i64, chunk_size: f64) -> Self {
        if min > max {
            return Self::empty();
        }
        let chunk = normalize_non_empty(chunk_size) as u64;
        let (first, last) = (min as u64, max as u64);
        let mut start = first;
        let mut done = false;
        Self::from_pull(move || {
            if done {
                return Effect::succeed(None);
            }
            let remaining = last - start + 1;
            let (take, finished) = if remaining > chunk {
                (chunk, false)
            } else {
                (remaining, true)
            };
            // `take <= remaining`, so `start + take <= last + 1` cannot overflow.
            let end = start + take;
            let out: Vec<i64> = ((start as i64)..(end as i64)).collect();
            start = end;
            done = finished;
            Effect::succeed(Some(out))
        })
    }
}

/// Stream an iterator in bounded chunks without materializing it.
pub fn from_iter<T, E, I>(iter: I, chunk_size: usize) -> Stream<T, E, ()>
where
    T: Send + 'static,
    E: Send + 'static,
    I: Iterator<Item = T> + Send + 'static,
{
    let iter = Arc::new(Mutex::new(iter));
    let chunk = chunk_size.max(1);
    Stream::from_pull(move || {
        let mut guard = iter.lock().unwrap();
        let mut out = Vec::with_capacity(chunk);
        for _ in 0..chunk {
            match guard.next() {
                Some(item) => out.push(item),
                None => break,
            }
        }
        Effect::succeed(if out.is_empty() { None } else { Some(out) })
    })
}

impl<A, E> Stream<A, E, ()>
where
    A: Send + 'static,
    E: Send + 'static,
{
    /// Fold every element into state, running pulls to completion.
    pub fn run_fold<S, F>(self, init: S, f: F) -> Exit<S, E>
    where
        F: Fn(S, A) -> S,
    {
        let mut state = init;
        loop {
            match call(&self.pull).run_sync() {
                Ok(None) => return Exit::Success(state),
                Ok(Some(items)) => {
                    for item in items {
                        state = f(state, item);
                    }
                }
                Err(cause) => return Exit::Failure(cause),
            }
        }
    }

    /// Collect every element in order.
    pub fn run_collect(self) -> Exit<Vec<A>, E> {
        self.run_fold(Vec::new(), |mut out, item| {
            out.push(item);
            out
        })
    }

    /// Collect chunks as emitted, preserving pull boundaries. This is the
    /// observable `Stream.chunks().runCollect()`: an element-level fold
    /// cannot see the boundaries a chunked pull produces.
    pub fn run_collect_chunks(self) -> Exit<Vec<Vec<A>>, E> {
        let mut out = Vec::new();
        loop {
            match call(&self.pull).run_sync() {
                Ok(None) => return Exit::Success(out),
                Ok(Some(items)) => out.push(items),
                Err(cause) => return Exit::Failure(cause),
            }
        }
    }

    /// `Stream.runForEachWhile` at the pin: visit every element, stopping at
    /// the first whose predicate returns false. That element IS visited
    /// (the predicate's own effect runs first); everything after it is not.
    /// Returns unit on normal completion or a false stop, like the pin's
    /// `Effect<void>`.
    pub fn run_for_each_while<F>(self, f: F) -> Exit<(), E>
    where
        F: Fn(A) -> bool,
    {
        let mut stop = false;
        loop {
            if stop {
                return Exit::Success(());
            }
            match call(&self.pull).run_sync() {
                Ok(None) => return Exit::Success(()),
                Ok(Some(items)) => {
                    for item in items {
                        if !f(item) {
                            stop = true;
                            break;
                        }
                    }
                }
                Err(cause) => return Exit::Failure(cause),
            }
        }
    }
}

impl<A, E, R> Stream<A, E, R>
where
    A: Send + 'static,
    E: Send + 'static,
    R: crate::effect::Requirement,
{
    /// Map each element through an effect, with at most `bound` children in
    /// flight (at least one). The mapping runs at interpretation with the
    /// pull's scope, so `f` effects share the stream's requirements.
    /// Failures fail the pull fast: live siblings are interrupted and joined
    /// before the failure is returned. Bounds below one behave as one.
    pub fn map_effect<B, F>(self, f: F, bound: usize) -> Stream<B, E, R>
    where
        B: Send + 'static,
        F: Fn(A, &crate::Ctx) -> Effect<B, E, R> + Send + Sync + 'static,
    {
        let (pull, exec, f) = (self.pull, Arc::clone(&self.exec), Arc::new(f));
        let (pull0, exec0, f0) = (Arc::clone(&pull), Arc::clone(&exec), Arc::clone(&f));
        let node = move || {
            let (pull, exec, f) = (Arc::clone(&pull0), Arc::clone(&exec0), Arc::clone(&f0));
            Effect {
                step: crate::effect::Step::Suspend(Box::new(MapEffect {
                    pull,
                    f,
                    bound: bound.max(1),
                    exec,
                    marker: std::marker::PhantomData,
                })),
                services: crate::service::empty_services(),
                marker: std::marker::PhantomData,
            }
        };
        Self::derive(exec, node)
    }

    /// Interrupt and join every in-flight fan-out child, then clear the
    /// registry. Later pulls run normally.
    pub fn cancel(&self) {
        let mut registry = self.exec.inflight.lock().unwrap();
        for (_, handles) in registry.drain() {
            for handle in handles {
                handle.interrupt();
                handle.join();
            }
        }
    }
}

/// One bounded fan-out over a pulled chunk. Runs at interpretation with the
/// pull's scope; children are fibers so cancellation interrupts them.
struct MapEffect<
    A: Send + 'static,
    B: Send + 'static,
    E: Send + 'static,
    R: crate::effect::Requirement,
    F,
> {
    pull: Pull<A, E, R>,
    f: Arc<F>,
    bound: usize,
    exec: Arc<StreamExec>,
    marker: std::marker::PhantomData<fn() -> B>,
}

/// Spawn one fan-out child and register it under `batch` for cancel().
#[allow(clippy::too_many_arguments)]
fn spawn_child<A, B, E, R, F>(
    rt: &std::sync::Arc<crate::Runtime>,
    tx: &std::sync::mpsc::Sender<ChildDone<B, E>>,
    f: &Arc<F>,
    scope: &crate::service::Services,
    registry: &Arc<StreamExec>,
    batch: u64,
    index: usize,
    item: A,
) where
    A: Send + 'static,
    B: Send + 'static,
    E: Send + 'static,
    R: crate::effect::Requirement,
    F: Fn(A, &crate::Ctx) -> Effect<B, E, R> + Send + Sync + 'static,
{
    let (tx, f, scope) = (tx.clone(), Arc::clone(f), scope.clone());
    let handle = rt.fork_detach_with(
        move |ctx| {
            let exit = match f(item, ctx).run_sync_with(&scope) {
                Ok(b) => Exit::Success(b),
                Err(cause) => Exit::Failure(cause),
            };
            let _ = tx.send(match exit {
                Exit::Success(b) => ChildDone::Ok(index, b),
                Exit::Failure(cause) => ChildDone::Failed(cause),
            });
            Exit::<(), std::convert::Infallible>::succeed(())
        },
        crate::ForkOpts::immediate(),
    );
    registry
        .inflight
        .lock()
        .unwrap()
        .entry(batch)
        .or_default()
        .push(handle);
}

enum ChildDone<B, E> {
    Ok(usize, B),
    Failed(Cause<E>),
}

use crate::Cause;

impl<A, B, E, R, F> crate::effect::ErasedStep<Option<Vec<B>>, E> for MapEffect<A, B, E, R, F>
where
    A: Send + 'static,
    B: Send + 'static,
    E: Send + 'static,
    R: crate::effect::Requirement,
    F: Fn(A, &crate::Ctx) -> Effect<B, E, R> + Send + Sync + 'static,
{
    fn run_erased(
        self: Box<Self>,
        scope: &crate::service::Services,
    ) -> Result<Option<Vec<B>>, Cause<E>> {
        // Drain upstream (skipping empty chunks) with this pull's scope.
        let items = loop {
            match call(&self.pull).run_sync_with(scope) {
                Ok(None) => return Ok(None),
                Ok(Some(items)) if items.is_empty() => continue,
                Ok(Some(items)) => break items,
                Err(cause) => return Err(cause),
            }
        };
        let total = items.len();
        let bound = self.bound.min(total).max(1);
        let rt = StreamExec::runtime(&self.exec);
        let batch = self
            .exec
            .next_batch
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        // Register the batch before spawning so cancel() sees every child.
        self.exec.inflight.lock().unwrap().insert(batch, Vec::new());
        let (tx, rx) = std::sync::mpsc::channel::<ChildDone<B, E>>();
        let mut pending = items.into_iter().enumerate();
        for _ in 0..bound {
            if let Some((index, item)) = pending.next() {
                spawn_child(&rt, &tx, &self.f, scope, &self.exec, batch, index, item);
            }
        }
        let mut results: Vec<Option<B>> = (0..total).map(|_| None).collect();
        let mut finished = 0;
        let mut failure: Option<Cause<E>> = None;
        while finished < total {
            match rx.recv().expect("a child always answers") {
                ChildDone::Ok(index, b) => {
                    if failure.is_none() {
                        results[index] = Some(b);
                        if let Some((next, item)) = pending.next() {
                            spawn_child(&rt, &tx, &self.f, scope, &self.exec, batch, next, item);
                        }
                    }
                    finished += 1;
                }
                ChildDone::Failed(cause) => {
                    if failure.is_none() {
                        failure = Some(cause);
                        // Fail fast: interrupt every live sibling. Their exits
                        // still arrive below and are drained, not recorded.
                        let registry = self.exec.inflight.lock().unwrap();
                        if let Some(handles) = registry.get(&batch) {
                            for handle in handles {
                                handle.interrupt();
                            }
                        }
                    }
                    finished += 1;
                }
            }
        }
        // Reap every child (interrupted ones included) before returning, so
        // releases complete first. Handles are taken out of the registry, so
        // cancel() can never join one twice.
        let registered = self
            .exec
            .inflight
            .lock()
            .unwrap()
            .remove(&batch)
            .unwrap_or_default();
        for handle in registered {
            handle.join();
        }
        match failure {
            Some(cause) => Err(cause),
            None => Ok(Some(results.into_iter().flatten().collect())),
        }
    }
}
