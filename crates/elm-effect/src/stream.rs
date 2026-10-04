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

use std::sync::{Arc, Mutex};

use crate::{Effect, Exit};

/// A shared pull factory: rebuilt effects, one call at a time.
type Pull<A, E, R> = Arc<Mutex<Box<dyn FnMut() -> Effect<Option<Vec<A>>, E, R> + Send>>>;

/// A pull-based stream. Pulls yield bounded chunks; `None` ends the stream.
/// Empty chunks are skipped, never terminal.
pub struct Stream<A: Send + 'static, E: Send + 'static, R: crate::effect::Requirement> {
    pull: Pull<A, E, R>,
}

impl<A: Send + 'static, E: Send + 'static, R: crate::effect::Requirement> Clone
    for Stream<A, E, R>
{
    fn clone(&self) -> Self {
        Self {
            pull: Arc::clone(&self.pull),
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
        let (pull, f) = (self.pull, Arc::new(f));
        Stream::from_pull(move || {
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
        let (pull, f) = (self.pull, Arc::new(f));
        Stream::from_pull(move || {
            let f = Arc::clone(&f);
            call(&pull)
                .map(move |chunk| chunk.map(|items| items.into_iter().filter(|a| f(a)).collect()))
        })
    }

    /// Take at most `n` elements, then end.
    pub fn take(self, n: u64) -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        let (pull, remaining) = (self.pull, Arc::new(AtomicU64::new(n)));
        Stream::from_pull(move || {
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
        let (first, second, drained) = (
            self.pull,
            Arc::new(Mutex::new(other.pull)),
            Arc::new(AtomicBool::new(false)),
        );
        Stream::from_pull(move || {
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

/// Stream an iterator in bounded chunks without materializing it.
pub fn from_iter<T, I>(iter: I, chunk_size: usize) -> Stream<T, std::convert::Infallible, ()>
where
    T: Send + 'static,
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
}
