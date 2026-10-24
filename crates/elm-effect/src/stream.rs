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

use std::collections::VecDeque;
use std::convert::Infallible;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::{Cause, Defect, Effect, Exit};

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
    ///
    /// `n` normalizes like Effect 4's `Count.normalize`, whose empty is 0:
    /// `NaN` and non-positive counts become 0, a positive count floors. The
    /// Effect 4's count is a `number`, so `take(0.5)` takes nothing and `take(1.9)`
    /// takes one, exactly as `take_right` already normalizes.
    ///
    /// A zero normalized count ends the stream without pulling the upstream,
    /// which is the short-circuit Effect 4 asserts for both the integer zero
    /// and `NaN`.
    pub fn take(self, n: f64) -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        let count = normalize(n) as u64;
        let (pull, remaining, exec) = (
            self.pull,
            Arc::new(AtomicU64::new(count)),
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

    /// `Stream.zipLatest`: both sides' latest values, paired.
    ///
    /// The crate pulls each side once per output, so a pair appears as soon as
    /// each side answers a chunk, and the last element of a chunk is that
    /// side's latest. The stream ends when either side ends, and a failure on
    /// either side is the stream's failure.
    ///
    /// ponytail: lockstep, not push. Effect 4 emits on every arrival, so a side
    /// that produces two chunks between the other side's one emits two pairs
    /// there and one here. Grow a per-side pending queue if a row counts pairs.
    pub fn zip_latest<B>(self, other: Stream<B, E, R>) -> Stream<(A, B), E, R>
    where
        B: Send + 'static,
    {
        self.zip_latest_with(other, |left, right| (left, right))
    }

    /// [`zip_latest`](Self::zip_latest) with a combining function.
    pub fn zip_latest_with<B, C, F>(self, other: Stream<B, E, R>, f: F) -> Stream<C, E, R>
    where
        B: Send + 'static,
        C: Send + 'static,
        F: Fn(A, B) -> C + Send + Sync + 'static,
    {
        let exec = Arc::clone(&self.exec);
        let left = self.pull;
        let right = Arc::new(Mutex::new(other.pull));
        let f = Arc::new(f);
        // The last value each side answered since the previous pair. A pair
        // consumes both, so the next output is always built from fresh chunks.
        let latest: Arc<Mutex<(Option<A>, Option<B>)>> = Arc::new(Mutex::new((None, None)));
        Self::derive(exec, move || {
            let (left, right, f, latest) = (
                Arc::clone(&left),
                Arc::clone(&right),
                Arc::clone(&f),
                Arc::clone(&latest),
            );
            call(&left).flat_map(move |chunk| {
                let Some(mut items) = chunk else {
                    return Effect::succeed(None);
                };
                let Some(latest_left) = items.pop() else {
                    return Effect::succeed(Some(Vec::new()));
                };
                latest.lock().unwrap().0 = Some(latest_left);
                let (right, f, latest) = (Arc::clone(&right), Arc::clone(&f), Arc::clone(&latest));
                call(&right.lock().unwrap()).flat_map(move |chunk| {
                    let Some(mut items) = chunk else {
                        return Effect::succeed(None);
                    };
                    let Some(latest_right) = items.pop() else {
                        return Effect::succeed(Some(Vec::new()));
                    };
                    let pair = {
                        let mut guard = latest.lock().unwrap();
                        guard.1 = Some(latest_right);
                        (guard.0.take(), guard.1.take())
                    };
                    match pair {
                        (Some(left), Some(right)) => Effect::succeed(Some(vec![f(left, right)])),
                        // Both sides answer a chunk before a pair is built, so
                        // this is unreachable; ending beats pairing a side that
                        // never answered.
                        _ => Effect::succeed(None),
                    }
                })
            })
        })
    }

    /// `Stream.filterMap` in Effect 4: keep only elements that map to `Some`.
    /// Effect 4's `Filter` can fail, but the observable subset here is the
    /// pure `Option` case (a failure maps to `None`).
    pub fn filter_map<B, F>(self, f: F) -> Stream<B, E, R>
    where
        B: Send + 'static,
        F: Fn(A) -> Option<B> + Send + Sync + 'static,
    {
        let (pull, f, exec) = (self.pull, Arc::new(f), Arc::clone(&self.exec));
        Self::derive(exec, move || {
            let f = Arc::clone(&f);
            call(&pull).map(move |chunk| {
                chunk.map(|items| items.into_iter().filter_map(|a| f(a)).collect())
            })
        })
    }

    /// `Stream.scan` in Effect 4: emit the running fold, excluding the initial
    /// value. One output chunk per input chunk, like Effect 4's channel map.
    pub fn scan<S, F>(self, init: S, f: F) -> Stream<S, E, R>
    where
        S: Clone + Send + 'static,
        F: Fn(S, A) -> S + Send + Sync + 'static,
    {
        let (pull, f, state) = (self.pull, Arc::new(f), Arc::new(Mutex::new(init)));
        let exec = Arc::clone(&self.exec);
        Self::derive(exec, move || {
            let (pull, f, state) = (Arc::clone(&pull), Arc::clone(&f), Arc::clone(&state));
            call(&pull).map(move |chunk| {
                let mut st = state.lock().unwrap();
                chunk.map(|items| {
                    items
                        .into_iter()
                        .map(|a| {
                            let next = f(st.clone(), a);
                            *st = next.clone();
                            next
                        })
                        .collect()
                })
            })
        })
    }

    /// `Stream.scanEffect` in Effect 4: the running fold with an effectful
    /// step. The step reads and updates one accumulator, so it rides
    /// `map_effect` with a bound of one: one child at a time is what keeps the
    /// fold sequential, and the state lives in the closure rather than in a
    /// second pull node.
    pub fn scan_effect<S, F>(self, init: S, f: F) -> Stream<S, E, R>
    where
        S: Clone + Send + 'static,
        F: Fn(S, A) -> Effect<S, E, R> + Send + Sync + 'static,
    {
        let (state, step) = (Arc::new(Mutex::new(init)), Arc::new(f));
        self.map_effect(
            move |a: A, _ctx: &crate::Ctx| {
                let (state, step) = (Arc::clone(&state), Arc::clone(&step));
                let acc = state.lock().unwrap().clone();
                step(acc, a).map(move |next| {
                    *state.lock().unwrap() = next.clone();
                    next
                })
            },
            1,
        )
    }

    /// `Stream.mapAccumArray` in Effect 4: thread state across chunks, emitting
    /// zero or more values per chunk. The mapping runs once per chunk.
    pub fn map_accum_array<S, B, F>(self, init: S, f: F) -> Stream<B, E, R>
    where
        S: Clone + Send + 'static,
        B: Send + 'static,
        F: Fn(S, Vec<A>) -> (S, Vec<B>) + Send + Sync + 'static,
    {
        let (pull, f, state) = (self.pull, Arc::new(f), Arc::new(Mutex::new(init)));
        let exec = Arc::clone(&self.exec);
        Self::derive(exec, move || {
            let (pull, f, state) = (Arc::clone(&pull), Arc::clone(&f), Arc::clone(&state));
            call(&pull).map(move |chunk| {
                let mut st = state.lock().unwrap();
                chunk.map(|items| {
                    let (next, values) = f(st.clone(), items);
                    *st = next;
                    values
                })
            })
        })
    }

    /// `Stream.buffer`: hold produced elements so the producer runs ahead.
    ///
    /// The capacity and the strategy are only observable when the source can
    /// run AHEAD of the fold. A pull source is pulled exactly once per output,
    /// so there is no producer to suspend, drop from, or slide: the transform
    /// is the identity. Whin Effect 4's buffered rows still assert, element
    /// order and the failure travelling through, holds exactly.
    ///
    /// ponytail: identity. Feed a bounded queue from a worker thread if a row
    /// ever needs the producer decoupled.
    pub fn buffer(self, _capacity: f64) -> Self {
        self
    }

    /// [`buffer`](Self::buffer) with Effect 4's `strategy` spelled out. A pull
    /// source never fills the buffer, so the strategy has nothing to act on.
    pub fn buffer_with(self, _strategy: crate::sync::Strategy, _capacity: f64) -> Self {
        self
    }

    /// `Stream.bufferArray`: the same buffering at ARRAY granularity, where the
    /// capacity is how many elements each output chunk carries. A pull source
    /// has no producer to run ahead either, but the chunk size IS observable,
    /// so this re-chunks.
    pub fn buffer_array(self, capacity: f64) -> Self {
        self.rechunk(capacity)
    }

    /// [`buffer_array`](Self::buffer_array) with Effect 4's `strategy` spelled
    /// out, which a pull source never reaches.
    pub fn buffer_array_with(self, _strategy: crate::sync::Strategy, capacity: f64) -> Self {
        self.rechunk(capacity)
    }

    /// `Stream.takeWhile` in Effect 4: take while `f` holds. The element that
    /// failed is NOT taken, and nothing after it is seen. A chunk that
    /// becomes empty without a stop pumps again, like Effect 4's `pump`.
    pub fn take_while<F>(self, f: F) -> Self
    where
        F: Fn(&A) -> bool + Send + Sync + 'static,
    {
        let f: Arc<dyn Fn(&A) -> bool + Send + Sync> = Arc::new(f);
        let (pull, done) = (self.pull, Arc::new(AtomicBool::new(false)));
        let exec = Arc::clone(&self.exec);
        let (p, d) = (Arc::clone(&pull), Arc::clone(&done));
        Self::derive(exec, move || {
            take_while_pump(Arc::clone(&p), Arc::clone(&f), Arc::clone(&d))
        })
    }

    /// `Stream.takeUntil` in Effect 4: take until `f` holds. The element that
    /// satisfied `f` IS taken (the default; Effect 4's `excludeLast` option
    /// would drop it), then the stream ends.
    pub fn take_until<F>(self, f: F) -> Self
    where
        F: Fn(&A) -> bool + Send + Sync + 'static,
    {
        let f: Arc<dyn Fn(&A) -> bool + Send + Sync> = Arc::new(f);
        let (pull, done) = (self.pull, Arc::new(AtomicBool::new(false)));
        let exec = Arc::clone(&self.exec);
        let (p, d) = (Arc::clone(&pull), Arc::clone(&done));
        Self::derive(exec, move || {
            take_until_pump(Arc::clone(&p), Arc::clone(&f), Arc::clone(&d))
        })
    }

    /// `Stream.takeWhileFilter` in Effect 4: keep the prefix the filter accepts,
    /// mapping each accepted element through the filter. A filter failure ends
    /// the stream and drops the failing element, which is Effect 4's
    /// `Result.failVoid` arm.
    pub fn take_while_filter<B, F>(self, f: F) -> Stream<B, E, R>
    where
        B: Send + 'static,
        F: Fn(&A) -> Option<B> + Send + Sync + 'static,
    {
        let f: Arc<dyn Fn(&A) -> Option<B> + Send + Sync> = Arc::new(f);
        let (pull, done) = (self.pull, Arc::new(AtomicBool::new(false)));
        let exec = Arc::clone(&self.exec);
        let (p, d) = (Arc::clone(&pull), Arc::clone(&done));
        Self::derive(exec, move || {
            take_while_filter_pump(Arc::clone(&p), Arc::clone(&f), Arc::clone(&d))
        })
    }

    /// `Stream.rechunk` in Effect 4: re-emit the upstream in chunks of exactly
    /// `target`, carrying a partial accumulator across pull boundaries.
    ///
    /// `target` normalizes like Effect 4's `Count.normalizeNonEmpty`, whose
    /// empty is 1, so `NaN` and non-positive still produce a usable chunk size
    /// rather than an infinite loop.
    ///
    /// Effect 4's own two shapes are carried: a source chunk that already is
    /// exactly `target` passes through untouched, and otherwise elements
    /// accumulate into a buffer of `target` whose leftover is emitted at the
    /// end of the stream. That leftover rule is what makes a target larger
    /// than the whole input yield a single chunk.
    pub fn rechunk(self, target: f64) -> Self {
        let target = normalize_non_empty(target).max(1);
        let (pull, exec) = (self.pull, Arc::clone(&self.exec));
        let carry = Arc::new(Mutex::new(Vec::<A>::new()));
        Self::derive(exec, move || {
            loop {
                let mut buffer = carry.lock().unwrap();
                if buffer.len() >= target {
                    let out: Vec<A> = buffer.drain(..target).collect();
                    return Effect::succeed(Some(out));
                }
                match call(&pull).run_sync() {
                    // The upstream ended: the leftover is the last chunk,
                    // unless there is nothing left to emit.
                    Ok(None) => {
                        if buffer.is_empty() {
                            return Effect::succeed(None);
                        }
                        let out = std::mem::take(&mut *buffer);
                        return Effect::succeed(Some(out));
                    }
                    Ok(Some(items)) if items.is_empty() => continue,
                    Ok(Some(items)) => {
                        buffer.extend(items);
                        continue;
                    }
                    Err(cause) => return Effect::fail_cause(cause),
                }
            }
        })
    }

    /// `Stream.onStart` in Effect 4: run `f` once when collection begins, before
    /// the upstream is pulled. The elements are untouched.
    ///
    /// Effect 4's signature widens the error channel (`E | E2`); this crate takes
    /// an infallible closure instead, which is exactly whin Effect 4's rows
    /// exercise (`Effect.sync`), and avoids the widening problem that blocked
    /// `takeUntilEffect`. An effectful `onStart` would need that machinery.
    ///
    /// The `Option`-take is what makes it once-only: the pump is `FnMut`, so it
    /// runs again on every pull, and `take` consumes `F` on the first call. A
    /// later call finds the slot empty and delegates straight to the upstream,
    /// which is Effect 4's semantics (the callback is not re-run).
    pub fn on_start<F>(self, f: F) -> Self
    where
        F: FnOnce() + Send + 'static,
    {
        let (pull, exec) = (self.pull, Arc::clone(&self.exec));
        let pending = Arc::new(Mutex::new(Some(f)));
        Self::derive(exec, move || {
            let (pull, pending) = (Arc::clone(&pull), Arc::clone(&pending));
            if let Some(f) = pending.lock().unwrap().take() {
                f();
            }
            call(&pull)
        })
    }

    /// `Stream.onEnd` in Effect 4: run `f` once when the upstream is exhausted,
    /// after its last chunk has been emitted.
    ///
    /// Same infallible-closure contract as [`Stream::on_start`], and for the
    /// same reason: both reference rows use `Effect.sync`.
    ///
    /// `f` is consumed when the pull returns `None`, so the upstream ending is
    /// what triggers it and a re-pull after the end does not re-run it.
    pub fn on_end<F>(self, f: F) -> Self
    where
        F: FnOnce() + Send + 'static,
    {
        let (pull, exec) = (self.pull, Arc::clone(&self.exec));
        let finished = Arc::new(Mutex::new(Some(f)));
        Self::derive(exec, move || {
            let (pull, finished) = (Arc::clone(&pull), Arc::clone(&finished));
            call(&pull).map(move |chunk| {
                if chunk.is_none() {
                    // The upstream ended, so this is the last pull.
                    if let Some(f) = finished.lock().unwrap().take() {
                        f();
                    }
                }
                chunk
            })
        })
    }

    /// `Stream.grouped` in Effect 4: `chunks` over `rechunk(self, n)`, so each
    /// group is emitted as one element and the chunk-size normalization is
    /// shared with `rechunk`.
    pub fn grouped(self, n: f64) -> Stream<Vec<A>, E, R> {
        let target = normalize_non_empty(n).max(1);
        let (pull, exec) = (self.pull, Arc::clone(&self.exec));
        let carry = Arc::new(Mutex::new(Vec::<A>::new()));
        Self::derive(exec, move || {
            let (pull, carry) = (Arc::clone(&pull), Arc::clone(&carry));
            loop {
                let mut buffer = carry.lock().unwrap();
                if buffer.len() >= target {
                    let out: Vec<A> = buffer.drain(..target).collect();
                    return Effect::succeed(Some(vec![out]));
                }
                match call(&pull).run_sync() {
                    Ok(None) => {
                        if buffer.is_empty() {
                            return Effect::succeed(None);
                        }
                        let out = std::mem::take(&mut *buffer);
                        return Effect::succeed(Some(vec![out]));
                    }
                    Ok(Some(items)) if items.is_empty() => continue,
                    Ok(Some(items)) => {
                        buffer.extend(items);
                        continue;
                    }
                    Err(cause) => return Effect::fail_cause(cause),
                }
            }
        })
    }

    /// `Stream.takeRight` in Effect 4: keep the last `n` elements, in order.
    /// `n` normalizes like Effect 4's `Count.normalize`, whose empty is 0.
    ///
    /// The whole upstream is drained before anything is emitted, so this
    /// holds a ring of at most `n` elements. That is also why a ZERO count is
    /// handled by returning [`Stream::empty`] rather than by draining and
    /// emitting nothing: Effect 4's "zero normalized counts do not evaluate
    /// the upstream" row asserts the upstream is never pulled at all, which a
    /// draining implementation could not honor.
    pub fn take_right(self, n: f64) -> Self {
        let count = normalize(n);
        if count == 0 {
            // Drop the source without pulling it.
            return Stream::<A, E, R>::empty();
        }
        let (pull, exec) = (self.pull, Arc::clone(&self.exec));
        let state = Arc::new(Mutex::new(TakeRight::new(count)));
        let (pull0, state0) = (Arc::clone(&pull), Arc::clone(&state));
        Self::derive(exec, move || {
            let (pull, state) = (Arc::clone(&pull0), Arc::clone(&state0));
            take_right_pump(pull, state)
        })
    }
}

/// `Stream.decodeText` in Effect 4: decode UTF-8 bytes to text, holding an
/// incomplete trailing sequence until a later pull completes it.
///
/// Effect 4 preserves the failure channel `E`, so an invalid or truncated
/// sequence is carried in `E` through the same bound `channel::decode_text`
/// uses rather than in a fresh error type.
impl<E, R> Stream<u8, E, R>
where
    E: From<std::string::FromUtf8Error> + Send + 'static,
    R: crate::effect::Requirement,
{
    /// Effect 4's `mkString` is a fold over the chunks joined with `""`, which
    /// for this crate is `run_collect` then `concat`.
    pub fn decode_text(self) -> Stream<String, E, R> {
        let (pull, exec) = (self.pull, Arc::clone(&self.exec));
        let carry = Arc::new(Mutex::new(Vec::<u8>::new()));
        Self::derive(exec, move || {
            loop {
                let mut buffer = carry.lock().unwrap();
                let text = drain_valid_utf8_prefix(&mut buffer);
                if !text.is_empty() {
                    return Effect::succeed(Some(vec![text]));
                }
                match call(&pull).run_sync() {
                    Ok(None) => {
                        // The upstream ended. An empty buffer is a clean end;
                        // anything left is a truncated sequence or invalid
                        // bytes, carried in E rather than emitted as a
                        // replacement character.
                        if buffer.is_empty() {
                            return Effect::succeed(None);
                        }
                        match String::from_utf8(std::mem::take(&mut buffer)) {
                            Ok(text) => return Effect::succeed(Some(vec![text])),
                            Err(error) => return Effect::fail(error.into()),
                        }
                    }
                    Ok(Some(items)) => {
                        buffer.extend(items);
                        continue;
                    }
                    Err(cause) => return Effect::fail_cause(cause),
                }
            }
        })
    }
}

/// Drain the longest valid UTF-8 prefix of `buffer`, leaving an incomplete
/// trailing sequence in place. Strict, never lossy.
///
/// This is the same boundary rule `channel::decode_text` needs, but
/// `channel.rs` is a `#[path]`-included test-only module, so it is a separate
/// compilation unit and the helper cannot be shared across it.
fn drain_valid_utf8_prefix(buffer: &mut Vec<u8>) -> String {
    let valid_up_to = match std::str::from_utf8(buffer) {
        Ok(_) => buffer.len(),
        Err(error) => error.valid_up_to(),
    };
    String::from_utf8(buffer.drain(..valid_up_to).collect()).expect("valid prefix")
}

impl<A: Send + 'static, E: Send + 'static, R: crate::effect::Requirement> Stream<A, E, R> {
    /// Clone this stream's pull factory (the shared `Arc`).
    ///
    /// Used by [`Stream::catch`] to keep the replacement stream in the shared
    /// state while still handing over a first chunk: the pull is an `Arc`, so
    /// cloning it lets the stream itself move into the state.
    pub(crate) fn clone_of_pull(&self) -> Pull<A, E, R> {
        Arc::clone(&self.pull)
    }

    /// `Stream.catch` in Effect 4: when this stream fails with a typed error,
    /// continue instead with the stream `handler` returns (observing that
    /// error). Defects and interruptions are NOT caught and pass through
    /// untouched.
    ///
    /// The failure never appears inside the pull's continuation (flat_map only
    /// sees successful chunks), so the catch is applied at the Effect level,
    /// where `Effect::catch` already fires only on a clean single `Fail`.
    pub fn catch<F>(self, handler: F) -> Self
    where
        F: FnOnce(E) -> Self + Send + 'static,
    {
        let handler: Arc<Mutex<Option<F>>> = Arc::new(Mutex::new(Some(handler)));
        // Installed on the first failure; every later pull delegates to it.
        let substitute: Arc<Mutex<Option<Self>>> = Arc::new(Mutex::new(None));
        let (pull, exec) = (self.pull, Arc::clone(&self.exec));
        let (_p, h, s) = (
            Arc::clone(&pull),
            Arc::clone(&handler),
            Arc::clone(&substitute),
        );
        Self::derive(exec, move || {
            // Once substituted, the replacement owns every later pull.
            let delegate = { s.lock().unwrap().as_ref().map(Stream::clone_of_pull) };
            if let Some(pull) = delegate {
                return call(&pull);
            }
            let h = Arc::clone(&h);
            let s = Arc::clone(&s);
            call(&pull).catch(move |error| {
                let handler = h
                    .lock()
                    .unwrap()
                    .take()
                    .expect("catch handler ran on a second failure");
                let replacement = handler(error);
                // Keep the replacement for later pulls, and hand over its
                // first chunk now. Cloning the pull (an Arc) lets the
                // replacement move into the state.
                let first = Arc::clone(&replacement.pull);
                *s.lock().unwrap() = Some(replacement);
                call(&first)
            })
        })
    }
}

impl<A: Send + 'static, E: Send + 'static> Stream<A, E, ()> {
    /// `Stream.fail` in Effect 4: a stream whose pull fails with `error`, then
    /// ends. First pull fails; a later pull sees an already-ended stream.
    ///
    /// Own generics on its own `impl`, separate from the combinator block:
    /// `A` and `E` are free here because this CONSTRUCTS the stream rather
    /// than taking `self` (the same shape as `range`).
    pub fn fail(error: E) -> Self {
        let once: Arc<Mutex<Option<E>>> = Arc::new(Mutex::new(Some(error)));
        Stream::from_pull(move || match once.lock().unwrap().take() {
            Some(error) => Effect::<Option<Vec<A>>, E>::fail(error),
            None => Effect::succeed(None::<Vec<A>>),
        })
    }
}

impl<E: Send + 'static, R: crate::effect::Requirement> Stream<String, E, R> {
    /// `Stream.splitLines` in Effect 4: split a stream of strings into lines,
    /// handling `\n`, `\r`, and `\r\n` across chunk boundaries.
    ///
    /// Three pieces of state survive across pulls, exactly as Effect 4 keeps
    /// them inside its transform closure:
    ///
    /// - `carry`: text not yet terminated by a line break, prepended to the
    ///   next line that is emitted.
    /// - `mid_crlf`: a chunk ended on `\r`. The next non-empty chunk's
    ///   leading `\n` is part of that same CRLF, not a second terminator.
    /// - `ended`: the upstream signalled completion, so a later pull returns
    ///   end without pulling again (the "does not pull upstream again after
    ///   flushing the final line" row).
    ///
    /// Byte indices are used deliberately: `\r` and `\n` are ASCII, and every
    /// computed start lands on a char boundary, so this is UTF-8 safe.
    pub fn split_lines(self) -> Self {
        let (pull, exec) = (self.pull, Arc::clone(&self.exec));
        let state = Arc::new(Mutex::new(SplitLinesState::default()));
        let (pull0, state0) = (Arc::clone(&pull), Arc::clone(&state));
        Self::derive(exec, move || {
            let (pull, state) = (Arc::clone(&pull0), Arc::clone(&state0));
            split_lines_pump(pull, state)
        })
    }
}

/// Live accumulator for [`Stream::split_lines`].
#[derive(Default)]
struct SplitLinesState {
    carry: String,
    mid_crlf: bool,
    ended: bool,
}

/// One `split_lines` pull: pull upstream, split what arrived, and only pull
/// again when the chunk produced no complete line (Effect 4's `pullOrFlush`).
fn split_lines_pump<E, R>(
    pull: Pull<String, E, R>,
    state: Arc<Mutex<SplitLinesState>>,
) -> Effect<Option<Vec<String>>, E, R>
where
    E: Send + 'static,
    R: crate::effect::Requirement,
{
    if state.lock().unwrap().ended {
        return Effect::succeed(None);
    }
    call(&pull).flat_map(move |chunk| {
        let (pull, state) = (Arc::clone(&pull), Arc::clone(&state));
        match chunk {
            None => {
                let mut guard = state.lock().unwrap();
                if !guard.carry.is_empty() {
                    let last = std::mem::take(&mut guard.carry);
                    guard.mid_crlf = false;
                    guard.ended = true;
                    return Effect::succeed(Some(vec![last]));
                }
                guard.ended = true;
                Effect::succeed(None)
            }
            Some(items) => {
                let mut out: Vec<String> = Vec::new();
                {
                    let mut guard = state.lock().unwrap();
                    for text in items {
                        split_one(&mut guard, &text, &mut out);
                    }
                }
                if out.is_empty() {
                    // Nothing terminated yet: pull again.
                    split_lines_pump(pull, state)
                } else {
                    Effect::succeed(Some(out))
                }
            }
        }
    })
}

/// Split one chunk, appending complete lines to `out` and carrying the
/// unterminated tail in the shared state.
///
/// Indices are absolute positions in `text`. `str::find` on a *slice*
/// returns an offset relative to that slice, while Effect 4's
/// `indexOf(needle, from)` is absolute — so every search after `from`
/// advances goes through [`find_from`], never a bare `text[from..].find`.
fn split_one(state: &mut SplitLinesState, text: &str, out: &mut Vec<String>) {
    if text.is_empty() {
        return;
    }
    let len = text.len();
    let mut from = 0usize;
    let mut cr = text.find('\r');
    let mut lf = text.find('\n');

    if state.mid_crlf {
        if lf == Some(0) {
            from = 1;
            lf = find_from(text, from, '\n');
        }
        state.mid_crlf = false;
    }

    while cr.is_some() || lf.is_some() {
        if cr.is_none() || (lf.is_some() && lf.unwrap() < cr.unwrap()) {
            let end = lf.unwrap();
            push_line(state, text, from, end, out);
            from = end + 1;
            lf = find_from(text, from, '\n');
        } else {
            let at = cr.unwrap();
            push_line(state, text, from, at, out);
            if at + 1 == len {
                // The chunk ends on `\r`: the terminator completes the line,
                // but a following `\n` belongs to this same CRLF.
                state.mid_crlf = true;
                from = len;
                cr = None;
            } else {
                from = at + if lf == Some(at + 1) { 2 } else { 1 };
                cr = find_from(text, from, '\r');
                lf = find_from(text, from, '\n');
            }
        }
    }
    state.carry.push_str(&text[from..]);
}

/// `text[from..].find(needle)`, returned as an ABSOLUTE index in `text`.
fn find_from(text: &str, from: usize, needle: char) -> Option<usize> {
    text[from..].find(needle).map(|offset| from + offset)
}

/// Emit one complete line: the carried prefix plus this segment, clearing the
/// carry (Effect 4's `pushLine`).
fn push_line(
    state: &mut SplitLinesState,
    text: &str,
    from: usize,
    to: usize,
    out: &mut Vec<String>,
) {
    let segment = &text[from..to];
    if state.carry.is_empty() {
        out.push(segment.to_string());
    } else {
        state.carry.push_str(segment);
        out.push(std::mem::take(&mut state.carry));
    }
}

/// `takeWhile`'s pull step: pull once, keep the prefix that satisfies `f`,
/// stop at the first miss. `done` is shared so a later factory call ends.
fn take_while_filter_pump<A, B, E, R>(
    pull: Pull<A, E, R>,
    f: Arc<dyn Fn(&A) -> Option<B> + Send + Sync>,
    done: Arc<AtomicBool>,
) -> Effect<Option<Vec<B>>, E, R>
where
    A: Send + 'static,
    B: Send + 'static,
    E: Send + 'static,
    R: crate::effect::Requirement,
{
    if done.load(Ordering::SeqCst) {
        return Effect::succeed(None);
    }
    call(&pull).flat_map(move |chunk| {
        match chunk {
            None => Effect::succeed(None),
            Some(items) => {
                let mut out: Vec<B> = Vec::new();
                let mut stop = false;
                for a in items {
                    match f(&a) {
                        Some(b) => out.push(b),
                        None => {
                            stop = true;
                            break;
                        }
                    }
                }
                if stop {
                    done.store(true, Ordering::SeqCst);
                }
                if out.is_empty() && stop {
                    Effect::succeed(None)
                } else {
                    // An empty `out` without a filter failure is Effect 4's
                    // `pump` loop: ask upstream for more. Matches
                    // `take_while`'s convention for an accepted-but-empty chunk.
                    Effect::succeed(Some(out))
                }
            }
        }
    })
}

fn take_while_pump<A, E, R>(
    pull: Pull<A, E, R>,
    f: Arc<dyn Fn(&A) -> bool + Send + Sync>,
    done: Arc<AtomicBool>,
) -> Effect<Option<Vec<A>>, E, R>
where
    A: Send + 'static,
    E: Send + 'static,
    R: crate::effect::Requirement,
{
    if done.load(Ordering::SeqCst) {
        return Effect::succeed(None);
    }
    call(&pull).flat_map(move |chunk| {
        let (_pull, f, done) = (Arc::clone(&pull), Arc::clone(&f), Arc::clone(&done));
        match chunk {
            None => Effect::succeed(None),
            Some(items) => {
                let mut out: Vec<A> = Vec::new();
                let mut stop = false;
                for a in items {
                    if f(&a) {
                        out.push(a);
                    } else {
                        stop = true;
                        break;
                    }
                }
                if stop {
                    done.store(true, Ordering::SeqCst);
                    if out.is_empty() {
                        Effect::succeed(None)
                    } else {
                        Effect::succeed(Some(out))
                    }
                } else {
                    // The whole chunk passed; the next pull asks upstream
                    // again. An empty chunk here is Effect 4's `pump`.
                    Effect::succeed(Some(out))
                }
            }
        }
    })
}

/// `takeUntil`'s pull step: pull once, and take through the first element
/// that satisfies `f` (inclusive). Before any match, the chunk passes.
fn take_until_pump<A, E, R>(
    pull: Pull<A, E, R>,
    f: Arc<dyn Fn(&A) -> bool + Send + Sync>,
    done: Arc<AtomicBool>,
) -> Effect<Option<Vec<A>>, E, R>
where
    A: Send + 'static,
    E: Send + 'static,
    R: crate::effect::Requirement,
{
    if done.load(Ordering::SeqCst) {
        return Effect::succeed(None);
    }
    call(&pull).flat_map(move |chunk| {
        let done = Arc::clone(&done);
        match chunk {
            None => Effect::succeed(None),
            Some(items) => match items.iter().position(|a| f(a)) {
                Some(index) => {
                    done.store(true, Ordering::SeqCst);
                    let mut out = items;
                    out.truncate(index + 1);
                    Effect::succeed(Some(out))
                }
                None => Effect::succeed(Some(items)),
            },
        }
    })
}

/// Mirror of Effect 4's `Count.normalizeNonEmpty` (`internal/count.ts`):
/// `NaN` and non-positive values become 1, a positive value floors, and
/// a positive infinity saturates to `usize::MAX` exactly as `Math.floor`
/// preserves it in TypeScript.
/// Mirror of Effect 4's `Count.normalize` (`internal/count.ts`), the sibling
/// of [`normalize_non_empty`]: `NaN` and non-positive values become 0, a
/// positive value floors. Effect 4 keeps these as two functions because a
/// chunk size and an element count have different empty behavior.
impl<A: Send + 'static> Stream<A, String, ()> {
    /// `Stream.fromQueue`: take from a queue as items arrive.
    ///
    /// The take BLOCKS, which is the whole point: a producer offering after the
    /// fold asked is what a queue decouples, and a pull source has no other way
    /// to see it. `end` and `shutdown` end the stream; a queue that failed
    /// fails it with the queue's message.
    ///
    /// The take runs inside the pull's effect, not when the stream is built, so
    /// describing the stream never blocks.
    pub fn from_queue(queue: Arc<crate::sync::Queue<A>>) -> Self {
        Stream::from_pull(move || {
            let queue = Arc::clone(&queue);
            Effect::sync(move || queue.take()).flat_map(|taken| match taken {
                Ok(item) => Effect::succeed(Some(vec![item])),
                Err(crate::sync::QueueError::Done) | Err(crate::sync::QueueError::Interrupted) => {
                    Effect::succeed(None)
                }
                Err(crate::sync::QueueError::Failed(message)) => Effect::fail(message),
            })
        })
    }
}

pub fn normalize(n: f64) -> usize {
    if n > 0.0 { n.floor() as usize } else { 0 }
}

pub fn normalize_non_empty(n: f64) -> usize {
    normalize(n).max(1)
}

impl Stream<i64, Infallible, ()> {
    /// `Stream.range` in Effect 4: inclusive integer range, chunked. `min` above
    /// `max` is empty; the chunk size normalizes like Effect 4's `Count`. Pulls
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

/// State for the byte limit pump.
struct LimitBytes<E, R>
where
    E: Send + 'static,
    R: crate::effect::Requirement,
{
    seen: i128,
    fallback: Option<Box<dyn FnOnce() -> Stream<Vec<u8>, E, R> + Send>>,
    switching: bool,
}

fn limit_bytes_pump<E, R>(
    pull: Pull<Vec<u8>, E, R>,
    state: Arc<Mutex<LimitBytes<E, R>>>,
    cap: i128,
) -> Effect<Option<Vec<Vec<u8>>>, E, R>
where
    E: Send + 'static,
    R: crate::effect::Requirement,
{
    let mut guard = state.lock().unwrap();
    if guard.switching {
        // Already past the limit: every pull delegates to the fallback.
        // It is built once, on the first pull that needs it.
        let Some(build) = guard.fallback.take() else {
            return Effect::succeed(None);
        };
        drop(guard);
        let fallback = build();
        return call(&Arc::clone(&fallback.pull));
    }
    match call(&pull).run_sync() {
        Ok(None) => Effect::succeed(None),
        Ok(Some(items)) => {
            let mut out: Vec<Vec<u8>> = Vec::with_capacity(items.len());
            for chunk in items {
                let would_be = guard.seen + chunk.len() as i128;
                if would_be > cap {
                    // Drop this chunk and everything after it, and switch.
                    guard.switching = true;
                    break;
                }
                guard.seen = would_be;
                out.push(chunk);
            }
            if !guard.switching {
                return if out.is_empty() {
                    Effect::succeed(None)
                } else {
                    Effect::succeed(Some(out))
                };
            }
            // The crossing chunk is dropped and the fallback replaces the rest
            // of the stream, so its first chunk is emitted in the SAME pull
            // after the passing prefix. Taking the fallback here means it is
            // built exactly once, and later pulls end rather than rebuild it.
            let Some(build) = guard.fallback.take() else {
                return Effect::succeed(None);
            };
            drop(guard);
            let fallback = build();
            match call(&Arc::clone(&fallback.pull)).run_sync() {
                Ok(Some(mut rest)) => {
                    out.append(&mut rest);
                    if out.is_empty() {
                        Effect::succeed(None)
                    } else {
                        Effect::succeed(Some(out))
                    }
                }
                Ok(None) => Effect::succeed(None),
                Err(cause) => Effect::fail_cause(cause),
            }
        }
        Err(cause) => Effect::fail_cause(cause),
    }
}

/// `Stream.callback` in Effect 4: a stream fed by a mailbox the registration
/// writes into.
///
/// The registration runs eagerly, before the first pull. That covers the
/// Effect 4's rows whose producer only offers and then signals end, failure, or
/// dies, none of which need the producer to run concurrently with the drain.
/// It does NOT cover Effect 4's backpressure and finalizer rows, whose
/// producer must block on a full mailbox while the consumer drains and whose
/// cleanup must fire on a fiber interrupt; both need a forked producer, which
/// this crate's `from_pull`-based streams do not have.
impl<A: Send + 'static, E: Send + 'static> Stream<A, E, ()> {
    pub fn callback<F>(register: F) -> Self
    where
        F: FnOnce(&mut Mailbox<A, E>),
    {
        let mut mailbox = Mailbox::new();
        register(&mut mailbox);
        let (items, terminator) = mailbox.into_parts();
        let chunk = items.len().max(1);
        match terminator {
            // No signal means the registration offered items and returned
            // without ending. Effect 4's stream is then unterminated and only a
            // bound such as `take` ends it. this crate has no non-terminating
            // primitive, so an unsignalled mailbox ends instead: the two differ
            // in general (a `take` larger than the offers would block in
            // Effect 4 and return what was offered here), which is a deliberate
            // divergence recorded rather than papered over. The test's own
            // case offers exactly as many as it takes, so both agree there.
            None | Some(Terminator::End) => crate::stream::from_iter(items.into_iter(), chunk),
            Some(Terminator::Fail(error)) => {
                let source = crate::stream::from_iter(items.into_iter(), chunk);
                source.concat(Self::fail(error))
            }
            Some(Terminator::Die(defect)) => {
                let source = crate::stream::from_iter(items.into_iter(), chunk);
                source.concat(Self::die(defect))
            }
        }
    }
}

/// The writer's side of [`Stream::callback`].
///
/// Items accumulate in order and the terminator records how the stream ends.
/// A registration that neither signals nor returns leaves the terminator unset,
/// which is Effect 4's "registration raised" case.
pub struct Mailbox<A, E> {
    items: Vec<A>,
    terminator: Option<Terminator<E>>,
}

/// How a mailbox closes.
pub enum Terminator<E> {
    /// Clean end of stream.
    End,
    /// A typed failure in the error channel.
    Fail(E),
    /// A defect, which is what a registration that raises becomes.
    Die(Defect),
}

impl<A, E> Mailbox<A, E> {
    /// An empty mailbox.
    pub fn new() -> Self {
        Self {
            items: Vec::new(),
            terminator: None,
        }
    }

    /// Offer one item.
    pub fn offer(&mut self, item: A) {
        self.items.push(item);
    }
    /// Offer many, in order.
    pub fn offer_all(&mut self, items: impl IntoIterator<Item = A>) {
        self.items.extend(items);
    }

    /// `Queue.endUnsafe`: end the stream cleanly.
    pub fn end(&mut self) {
        if self.terminator.is_none() {
            self.terminator = Some(Terminator::End);
        }
    }

    /// `Queue.failCauseUnsafe`: end the stream with a typed failure.
    pub fn fail(&mut self, error: E) {
        self.terminator = Some(Terminator::Fail(error));
    }

    /// End the stream with a defect, which is Effect 4's registration
    /// raising: the exit is `die`, not a typed failure in the error channel.
    pub fn die(&mut self, defect: Defect) {
        self.terminator = Some(Terminator::Die(defect));
    }

    fn into_parts(self) -> (Vec<A>, Option<Terminator<E>>) {
        (self.items, self.terminator)
    }
}

/// `Stream.mkArrayBuffer` in Effect 4: concatenate the byte chunks into one
/// buffer.
///
/// Effect 4 folds `mkUint8Array` and then projects `.buffer`; this crate has no
/// `ArrayBuffer` type, so the fold's accumulated bytes are the value. Declared
/// on the byte-element stream so `A` is not generic here, and built from
/// `run_fold`, which owns the same fold.
impl<E: Send + 'static> Stream<Vec<u8>, E, ()> {
    pub fn mk_array_buffer(self) -> Exit<Vec<u8>, E> {
        self.run_fold(Vec::new(), |mut acc, chunk| {
            acc.extend_from_slice(&chunk);
            acc
        })
    }

    /// `Stream.limitBytes` in Effect 4: pass byte chunks through while the
    /// running total stays within `limit`, and once a chunk would cross it,
    /// drop that chunk and replace the rest of the stream with
    /// `on_limit_reached`.
    ///
    /// The boundary is inclusive: a total equal to the limit still passes,
    /// which is whin Effect 4's comparison against the accumulated size means.
    /// The fallback is a `LazyArg`, so constructing it is observable and a
    /// caller can tell "never reached" from "reached", which two of the three
    /// rows assert.
    pub fn limit_bytes<F>(self, limit: i128, on_limit_reached: F) -> Self
    where
        F: FnOnce() -> Self + Send + 'static,
    {
        let (pull, exec) = (self.pull, Arc::clone(&self.exec));
        let state = Arc::new(Mutex::new(LimitBytes {
            seen: 0,
            fallback: Some(Box::new(on_limit_reached)),
            switching: false,
        }));
        let (p, s) = (Arc::clone(&pull), Arc::clone(&state));
        Self::derive(exec, move || {
            let (pull, state) = (Arc::clone(&p), Arc::clone(&s));
            limit_bytes_pump(pull, state, limit)
        })
    }
}

/// `Stream.fromIteratorSucceed` in Effect 4: a stream from an iterator whose
/// chunk size is a `number`, normalized by the same
/// `Count.normalizeNonEmpty` law `rechunk` and `grouped` use (NaN and
/// non-positive become 1, a positive value floors).
///
/// Effect 4 keeps this distinct from `fromIterable`, whose chunking arrives as
/// an option; this crate likewise keeps the `usize` `from_iter` for its
/// existing callers and normalizes here rather than widening all of them.
pub fn from_iterator_succeed<T, E, I>(iter: I, max_chunk_size: f64) -> Stream<T, E, ()>
where
    T: Send + 'static,
    E: Send + 'static,
    I: Iterator<Item = T> + Send + 'static,
{
    from_iter(iter, normalize_non_empty(max_chunk_size))
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

    /// `Stream.runForEachWhile` in Effect 4: visit every element, stopping at
    /// the first whose predicate returns false. That element IS visited
    /// (the predicate's own effect runs first); everything after it is not.
    /// Returns unit on normal completion or a false stop, like Effect 4's
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
        let mut spawned = bound;
        let mut finished = 0;
        let mut failure: Option<Cause<E>> = None;
        // Wait for the children that actually started. A failure stops the
        // pending queue from starting (their results are discarded anyway), so
        // waiting for `total` here would block forever on a recv that can
        // never arrive.
        while finished < spawned {
            match rx.recv().expect("a child always answers") {
                ChildDone::Ok(index, b) => {
                    if failure.is_none() {
                        results[index] = Some(b);
                        if let Some((next, item)) = pending.next() {
                            spawn_child(&rt, &tx, &self.f, scope, &self.exec, batch, next, item);
                            spawned += 1;
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

/// Live state for [`Stream::take_right`].
///
/// Generic in `A` because the ring stores the VALUES themselves: elements
/// are moved out of each pulled chunk into the ring, so no `Clone` bound is
/// imposed on `A` (an earlier attempt stored "positions" instead and needed
/// placeholder projections to recover them, which was fake scaffolding).
struct TakeRight<A> {
    /// At most `count` elements, oldest first, drained into output order.
    ring: VecDeque<A>,
    count: usize,
    /// The upstream signalled completion, so nothing further is pulled.
    drained: bool,
    /// The ring has been handed over; the stream is finished.
    flushed: bool,
}

impl<A> TakeRight<A> {
    fn new(count: usize) -> Self {
        Self {
            ring: VecDeque::with_capacity(count),
            count,
            drained: false,
            flushed: false,
        }
    }

    /// Keep `item`, dropping the oldest once the ring is full.
    fn observe(&mut self, item: A) {
        if self.ring.len() == self.count {
            self.ring.pop_front();
        }
        self.ring.push_back(item);
    }
}

/// One `take_right` pull: drain the upstream into the ring, then hand the
/// ring out once and end.
///
/// Written as a `flat_map` chain rather than a `run_sync` loop so that no
/// service requirement is imposed: an upstream failure short-circuits the
/// same chain, and an error is never re-raised with a constructor this
/// module does not own.
fn take_right_pump<A, E, R>(
    pull: Pull<A, E, R>,
    state: Arc<Mutex<TakeRight<A>>>,
) -> Effect<Option<Vec<A>>, E, R>
where
    A: Send + 'static,
    E: Send + 'static,
    R: crate::effect::Requirement,
{
    if state.lock().unwrap().flushed {
        return Effect::succeed(None);
    }
    call(&pull).flat_map(move |chunk| {
        let (pull, state) = (Arc::clone(&pull), Arc::clone(&state));
        match chunk {
            // Upstream ended: emit whatever survived in the ring.
            None => {
                let mut guard = state.lock().unwrap();
                guard.drained = true;
                guard.flushed = true;
                let out: Vec<A> = guard.ring.drain(..).collect();
                drop(guard);
                if out.is_empty() {
                    Effect::succeed(None)
                } else {
                    Effect::succeed(Some(out))
                }
            }
            // Observe every element, then pull again to drain the rest.
            Some(items) => {
                {
                    let mut guard = state.lock().unwrap();
                    for item in items {
                        guard.observe(item);
                    }
                }
                take_right_pump(pull, state)
            }
        }
    })
}

/// Live state for the catch-family (see [`Stream::catch`, [`Stream::catch_if`]).
///
/// Installed on the first caught failure; every later pull delegates to it.
/// Kept private to this module: the public surface is the combinators
/// themselves.
struct CatchState<A: Send + 'static, E: Send + 'static, R: crate::effect::Requirement> {
    substitute: std::sync::Arc<Mutex<Option<Stream<A, E, R>>>>,
    /// The handler arming, if any. `None` until a failure is caught.
    handler: std::sync::Arc<Mutex<Option<Box<dyn FnOnce(E) -> Stream<A, E, R> + Send>>>>,
}

impl<A: Send + 'static, E: Send + 'static, R: crate::effect::Requirement> Clone
    for CatchState<A, E, R>
{
    fn clone(&self) -> Self {
        Self {
            substitute: Arc::clone(&self.substitute),
            handler: Arc::clone(&self.handler),
        }
    }
}

/// Shared driver for the catch family, mirroring [`Stream::catch`]'s shape
/// but taking a predicate: an unhandled failure re-raises, a handled one
/// installs the handler's replacement stream.
fn catch_family_pump<A, E, R>(
    pull: Pull<A, E, R>,
    state: CatchState<A, E, R>,
    predicate: Arc<dyn Fn(&E) -> bool + Send + Sync>,
) -> Effect<Option<Vec<A>>, E, R>
where
    A: Send + 'static,
    E: Send + 'static,
    R: crate::effect::Requirement,
{
    let delegate = {
        state
            .substitute
            .lock()
            .unwrap()
            .as_ref()
            .map(Stream::clone_of_pull)
    };
    if let Some(sub) = delegate {
        return call(&sub);
    }
    let (state, predicate) = (state.clone(), Arc::clone(&predicate));
    call(&pull).catch(move |error| {
        // Take the handler ONCE. The earlier version took it for the
        // predicate check and then took it again with an expect(), which
        // panicked because the first take had already emptied the mutex.
        let arm = state.handler.lock().unwrap().take();
        // Unhandled when the handler is already spent, or the predicate
        // rejects this error. Put the handler back for a later pull and
        // re-raise the SAME typed failure, unchanged.
        let arm = match arm {
            Some(arm) if predicate(&error) => arm,
            maybe => {
                *state.handler.lock().unwrap() = maybe;
                return Effect::fail(error);
            }
        };
        let replacement = arm(error);
        // Clone the replacement's pull (an Arc) so the stream itself can move
        // into the shared state while we hand over its first chunk now.
        let first = Arc::clone(&replacement.pull);
        *state.substitute.lock().unwrap() = Some(replacement);
        call(&first)
    })
}

impl<A: Send + 'static, E: Send + 'static, R: crate::effect::Requirement> Stream<A, E, R> {
    /// `Stream.tapCause` in Effect 4: observe the whole cause, then re-raise
    /// it VERBATIM. `Fail` stays a typed failure and `Die` stays a defect,
    /// which is the property rows 943/953 assert.
    ///
    /// Applied at the Effect level via `catch_cause` (the failure never
    /// appears inside the pull's continuation), and re-raised with
    /// `Effect::fail_cause` so nothing is collapsed.
    pub fn tap_cause<F>(self, observer: F) -> Self
    where
        F: FnOnce(&crate::Cause<E>) + Send + 'static,
    {
        let (pull, exec) = (self.pull, Arc::clone(&self.exec));
        let observer = Arc::new(Mutex::new(Some(observer)));
        let (pull0, obs0) = (Arc::clone(&pull), Arc::clone(&observer));
        Self::derive(exec, move || {
            let observer = Arc::clone(&obs0);
            let pull = Arc::clone(&pull0);
            // Taking the observer out of the mutex IS the once-only guard: a
            // later pull finds it empty and re-raises without observing twice.
            call(&pull).catch_cause(move |cause| {
                if let Some(observe) = observer.lock().unwrap().take() {
                    observe(&cause);
                }
                Effect::fail_cause(cause)
            })
        })
    }

    /// `Stream.catchDefect` in Effect 4: recover from a DEFECT with
    /// `handler`, continuing with the stream it returns. A typed `Fail` is
    /// NOT a defect: it is re-raised unchanged, so `catchDefect` never
    /// swallows a typed failure.
    ///
    /// "Defect" here means a cause with at least one `Die` and no `Fail`,
    /// mirroring how Effect 4 keeps the two reason kinds apart.
    pub fn catch_defect<F>(self, handler: F) -> Self
    where
        F: FnOnce(Defect) -> Self + Send + 'static,
    {
        let (pull, exec) = (self.pull, Arc::clone(&self.exec));
        let substitute: Arc<Mutex<Option<Self>>> = Arc::new(Mutex::new(None));
        let handler = Arc::new(Mutex::new(Some(handler)));
        let (pull0, sub0, h0) = (
            Arc::clone(&pull),
            Arc::clone(&substitute),
            Arc::clone(&handler),
        );
        Self::derive(exec, move || {
            let delegate = { sub0.lock().unwrap().as_ref().map(Stream::clone_of_pull) };
            if let Some(sub) = delegate {
                return call(&sub);
            }
            let (handler, sub) = (Arc::clone(&h0), Arc::clone(&sub0));
            call(&pull0).catch_cause(move |cause| {
                // Only a defect-only cause is recoverable. Anything carrying a
                // Fail reason is re-raised verbatim.
                if cause.failures().is_empty() {
                    // defects() yields Vec<&Defect>, so first() is Option<&&Defect>
                    // and copied() one level short: clone once to own it.
                    if let Some(defect) = cause.defects().first().map(|d| (*d).clone()) {
                        let arm = handler.lock().unwrap().take();
                        if let Some(arm) = arm {
                            let replacement = arm(defect);
                            let first = Arc::clone(&replacement.pull);
                            *sub.lock().unwrap() = Some(replacement);
                            return call(&first);
                        }
                    }
                }
                Effect::fail_cause(cause)
            })
        })
    }

    /// `Stream.catchIf` in Effect 4: catch only failures satisfying
    /// `predicate`, continuing with the stream `handler` produces. A failure
    /// the predicate rejects is re-raised unchanged, so `catchIf` never
    /// accidentally recovers an error it does not name.
    ///
    /// Tag dispatch lives in the predicate, which is how Effect 4's
    /// `catchTag`/`catchReason` compose through ordinary closures.
    pub fn catch_if<P, F>(self, predicate: P, handler: F) -> Self
    where
        P: Fn(&E) -> bool + Send + Sync + 'static,
        F: FnOnce(E) -> Self + Send + 'static,
    {
        let state = CatchState {
            substitute: Arc::new(Mutex::new(None)),
            handler: Arc::new(Mutex::new(Some(Box::new(handler)))),
        };
        let (pull, exec) = (self.pull, Arc::clone(&self.exec));
        // Coerce to a trait object here: the pump takes Arc<dyn Fn(&E) -> bool>,
        // and an Arc<P> will not coerce implicitly inside the closure.
        let predicate: Arc<dyn Fn(&E) -> bool + Send + Sync> = Arc::new(predicate);
        let (pull0, state0, p0) = (Arc::clone(&pull), state.clone(), Arc::clone(&predicate));
        Self::derive(exec, move || {
            catch_family_pump(Arc::clone(&pull0), state0.clone(), Arc::clone(&p0))
        })
    }
}

impl<A: Send + 'static, E: Send + 'static, R: crate::effect::Requirement> Stream<A, E, R> {
    /// `Stream.catchFilter` in Effect 4: a `Result`-returning selector decides
    /// the recovery. `Ok(value)` is passed to `handler`, whose stream
    /// replaces the failure; `Err(failure)` is passed to `or_else`, whose
    /// stream replaces it instead. Both arms recover, so nothing is
    /// re-raised.
    ///
    /// This is the `orElse` shape the crate already has on `Channel`, with a
    /// `Result` selector in place of a boolean predicate.
    pub fn catch_filter<S, EF, H, G>(self, selector: EF, handler: H, or_else: G) -> Self
    where
        S: Send + 'static,
        EF: FnOnce(E) -> std::result::Result<S, String> + Send + 'static,
        H: FnOnce(S) -> Self + Send + 'static,
        G: FnOnce(String) -> Self + Send + 'static,
    {
        // Own state, not CatchState: the selector produces a type S that the
        // handler consumes, which CatchState's FnOnce(E) handler cannot
        // express. The failure channel is fixed at String here because that
        // is what the crate's selector maps to.
        let substitute: Arc<Mutex<Option<Self>>> = Arc::new(Mutex::new(None));
        let or_else: std::sync::Arc<Mutex<Option<Box<dyn FnOnce(String) -> Self + Send>>>> =
            std::sync::Arc::new(std::sync::Mutex::new(Some(Box::new(or_else))));
        let (pull, exec) = (self.pull, Arc::clone(&self.exec));
        // Both callables are FnOnce but the factory is FnMut, so each lives
        // behind a Mutex<Option<_>> and is taken when the failure arrives.
        let handler = Arc::new(Mutex::new(Some(handler)));
        let selector = Arc::new(Mutex::new(Some(selector)));
        let (pull0, sub0) = (Arc::clone(&pull), Arc::clone(&substitute));
        Self::derive(exec, move || {
            // Once substituted, the replacement owns every later pull.
            let delegate = { sub0.lock().unwrap().as_ref().map(Stream::clone_of_pull) };
            if let Some(sub) = delegate {
                return call(&sub);
            }
            let (selector, or_else) = (Arc::clone(&selector), Arc::clone(&or_else));
            let (handler, sub) = (Arc::clone(&handler), Arc::clone(&sub0));
            call(&pull0).catch(move |error| {
                let Some(select) = selector.lock().unwrap().take() else {
                    return Effect::fail(error);
                };
                let replacement = match select(error) {
                    std::result::Result::Ok(value) => {
                        handler.lock().unwrap().take().expect("checked")(value)
                    }
                    std::result::Result::Err(failure) => {
                        or_else.lock().unwrap().take().expect("checked")(failure)
                    }
                };
                let first = Arc::clone(&replacement.pull);
                *sub.lock().unwrap() = Some(replacement);
                call(&first)
            })
        })
    }
}

impl<A: Send + 'static, E: Send + 'static> Stream<A, E, ()> {
    /// `Stream.die` in Effect 4: a stream whose pull dies with `defect`, then
    /// ends. Distinct from [`Stream::fail`]: a defect is never a typed error,
    /// so `catch_if`/`catch` must not recover it.
    pub fn die(defect: Defect) -> Self {
        let once: Arc<Mutex<Option<Defect>>> = Arc::new(Mutex::new(Some(defect)));
        Stream::from_pull(move || match once.lock().unwrap().take() {
            Some(defect) => Effect::<Option<Vec<A>>, E>::die(defect),
            None => Effect::succeed(None::<Vec<A>>),
        })
    }
}

impl<A: Send + 'static, E: Clone + Send + 'static, R: crate::effect::Requirement> Stream<A, E, R> {
    /// `Stream.catchCauseFilter` in Effect 4: a cause-level extractor decides
    /// the recovery. `Ok(value)` is passed to `handler`, whose stream
    /// replaces the failure; `Err(cause)` RE-RAISES that cause unchanged, so
    /// a cause the extractor declines is preserved verbatim.
    ///
    /// Cause-level, not error-level: the extractor sees the whole cause
    /// (matching Effect 4's `Cause.hasFails ? Ok(squash) : Err(cause)` shape),
    /// which is what lets a non-typed reason reach the re-raise path instead
    /// of being collapsed into a typed failure.
    pub fn catch_cause_filter<V, EF, H>(self, extractor: EF, handler: H) -> Self
    where
        V: Send + 'static,
        EF: FnOnce(crate::Cause<E>) -> std::result::Result<V, crate::Cause<E>> + Send + 'static,
        H: FnOnce(V) -> Self + Send + 'static,
    {
        let (pull, exec) = (self.pull, Arc::clone(&self.exec));
        let substitute: Arc<Mutex<Option<Self>>> = Arc::new(Mutex::new(None));
        let extractor = Arc::new(Mutex::new(Some(extractor)));
        let handler = Arc::new(Mutex::new(Some(handler)));
        let (pull0, sub0) = (Arc::clone(&pull), Arc::clone(&substitute));
        let (ex0, h0) = (Arc::clone(&extractor), Arc::clone(&handler));
        Self::derive(exec, move || {
            let delegate = { sub0.lock().unwrap().as_ref().map(Stream::clone_of_pull) };
            if let Some(sub) = delegate {
                return call(&sub);
            }
            let (extractor, handler, sub) = (Arc::clone(&ex0), Arc::clone(&h0), Arc::clone(&sub0));
            call(&pull0).catch_cause(move |cause| {
                let Some(extract) = extractor.lock().unwrap().take() else {
                    return Effect::fail_cause(cause);
                };
                match extract(cause) {
                    std::result::Result::Ok(value) => {
                        let arm = handler.lock().unwrap().take().expect("checked");
                        let replacement = arm(value);
                        let first = Arc::clone(&replacement.pull);
                        *sub.lock().unwrap() = Some(replacement);
                        call(&first)
                    }
                    // Declined: put the cause back exactly as it came.
                    std::result::Result::Err(declined) => Effect::fail_cause(declined),
                }
            })
        })
    }
}
