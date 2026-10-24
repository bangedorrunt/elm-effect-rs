//! Pull-to-pull transforms: the stream conversation's middle.
//!
//! A channel maps each take and passes `End`/`Fail` through, so termination
//! and failure need no special handling downstream.
//!
//! Out of scope: the Effect source-channel constructors (`succeed`, `fail`,
//! `empty`, `never`, `from_effect`, `from_iterable`, `repeat`, `forever`, and
//! friends). This crate's `Channel` is a transform, so those belong to
//! `Pull`/`Effect`, and the ones that need runtime substrate (interruption,
//! ReadableStream) are not carried.

use super::pull::Pull;
use super::take::Take;
use super::{Cause, Defect};

/// A per-take step: one take in, zero or more out.
pub type StepFn<A, E, B> = Box<dyn FnMut(Take<A, E>) -> Vec<Take<B, E>> + Send>;

/// A transform from takes to takes, applied per pull.
pub struct Channel<A, E, B> {
    step: StepFn<A, E, B>,
}

impl<A, E, B> Channel<A, E, B>
where
    A: Send + 'static,
    E: Send + 'static,
    B: Send + 'static,
{
    /// Build from a per-take step (one take in, zero or more out).
    pub fn from_fn(step: impl FnMut(Take<A, E>) -> Vec<Take<B, E>> + Send + 'static) -> Self {
        Self {
            step: Box::new(step),
        }
    }

    /// Feed this channel's takes into `next` (`Channel.pipeTo`).
    ///
    /// Both steps run per take, so neither channel's state is reset: the
    /// composition is one channel, not a re-entry into `run`.
    pub fn pipe<C>(self, next: Channel<B, E, C>) -> Channel<A, E, C>
    where
        C: Send + 'static,
    {
        let mut first = self.step;
        let mut second = next.step;
        Channel::from_fn(move |take| {
            let mut out = Vec::new();
            for take in first(take) {
                out.extend(second(take));
            }
            out
        })
    }

    /// Drive `source` through this channel, yielding transformed takes.
    /// Every take (items, end, failure) flows through `step`: termination
    /// and failure handling belong to the channel, not the driver. If a
    /// terminal take produces nothing, the stream ends.
    pub fn run(self, source: Pull<A, E>) -> Pull<B, E> {
        let mut source = source;
        let mut step = self.step;
        // A queue, not a Vec: a channel that yields many takes per pull is
        // otherwise quadratic in `remove(0)` shifts.
        let mut pending: std::collections::VecDeque<Take<B, E>> = std::collections::VecDeque::new();
        Pull::from_fn(move || {
            loop {
                if let Some(take) = pending.pop_front() {
                    return take;
                }
                let take = source.pull();
                let terminal = !matches!(take, Take::Items(_));
                pending = step(take).into();
                if terminal && pending.is_empty() {
                    return Take::End;
                }
            }
        })
    }
}

/// Pass takes through unchanged.
pub fn identity<A, E>() -> Channel<A, E, A>
where
    A: Send + 'static,
    E: Send + 'static,
{
    Channel::from_fn(|take| vec![take])
}

/// Map items, lazily per pull. Free function so output-element inference
/// flows from the closure (a method would leave `Self` underdetermined).
pub fn map<A, E, B, F>(mut f: F) -> Channel<A, E, B>
where
    A: Send + 'static,
    E: Send + 'static,
    B: Send + 'static,
    F: FnMut(A) -> B + Send + 'static,
{
    Channel::from_fn(move |take| vec![take.map(&mut f)])
}

/// Map-then-collect, dropping `None`s per pull (`filterMap` shapes).
pub fn filter_map<A, E, B, F>(mut f: F) -> Channel<A, E, B>
where
    A: Send + 'static,
    E: Send + 'static,
    B: Send + 'static,
    F: FnMut(A) -> Option<B> + Send + 'static,
{
    Channel::from_fn(move |take| match take {
        Take::Items(items) => vec![Take::Items(items.into_iter().filter_map(&mut f).collect())],
        Take::End => vec![Take::End],
        Take::Fail(cause) => vec![Take::Fail(cause)],
    })
}

/// Recover failures passing a predicate (`catchFilter` shapes): matching
/// failures become `End` (the stream ends instead of failing).
pub fn recover<A, E, F>(mut predicate: F) -> Channel<A, E, A>
where
    A: Send + Clone + 'static,
    E: Send + 'static,
    F: FnMut(&E) -> bool + Send + 'static,
{
    Channel::from_fn(move |take| match take {
        // Only a cause that is nothing but typed failures may be recovered:
        // a defect or an interrupt is not the caller's `E`.
        Take::Fail(cause)
            if cause.is_typed_failure() && cause.failure().is_some_and(&mut predicate) =>
        {
            vec![Take::End]
        }
        other => vec![other],
    })
}
/// Map with fallible per-item functions (`mapEffect` shapes): an `Err`
/// item ends its pull with that failure.
pub fn map_effect<A, E, B, F>(mut f: F) -> Channel<A, E, B>
where
    A: Send + 'static,
    E: Send + 'static,
    B: Send + 'static,
    F: FnMut(A) -> Result<B, E> + Send + 'static,
{
    Channel::from_fn(move |take| match take {
        Take::Items(items) => {
            let mut out = Vec::with_capacity(items.len());
            for a in items {
                match f(a) {
                    Ok(b) => out.push(b),
                    Err(e) => return vec![Take::fail(e)],
                }
            }
            vec![Take::Items(out)]
        }
        Take::End => vec![Take::End],
        Take::Fail(cause) => vec![Take::Fail(cause)],
    })
}

/// Map each item with a fallible, filtering function (`filterMapEffect` shape):
/// an `Err` ends the pull with that failure; `Ok(None)` drops the item;
/// `Ok(Some(b))` keeps it. Runs synchronously like [`map_effect`].
pub fn filter_map_effect<A, E, B, F>(mut f: F) -> Channel<A, E, B>
where
    A: Send + 'static,
    E: Send + 'static,
    B: Send + 'static,
    F: FnMut(A) -> Result<Option<B>, E> + Send + 'static,
{
    Channel::from_fn(move |take| match take {
        Take::Items(items) => {
            let mut out = Vec::with_capacity(items.len());
            for a in items {
                match f(a) {
                    Ok(Some(b)) => out.push(b),
                    Ok(None) => {}
                    Err(e) => return vec![Take::fail(e)],
                }
            }
            vec![Take::Items(out)]
        }
        Take::End => vec![Take::End],
        Take::Fail(cause) => vec![Take::Fail(cause)],
    })
}

/// Concatenate per-item sub-pulls (`flatMap` shapes): each item maps to a
/// `Pull` that is driven to completion, takes concatenated in order. An
/// inner `Fail` fails the whole output; inner leftovers cannot occur
/// (inner pulls always run to `End`).
pub fn flat_map<A, E, B, F>(mut f: F) -> Channel<A, E, B>
where
    A: Send + 'static,
    E: Send + 'static,
    B: Send + 'static,
    F: FnMut(A) -> Pull<B, E> + Send + 'static,
{
    Channel::from_fn(move |take| match take {
        Take::Items(items) => {
            let mut out = Vec::new();
            for a in items {
                let mut inner = f(a);
                loop {
                    match inner.pull() {
                        Take::Items(mut chunk) => out.append(&mut chunk),
                        Take::End => break,
                        Take::Fail(cause) => return vec![Take::Fail(cause)],
                    }
                }
            }
            vec![Take::Items(out)]
        }
        Take::End => vec![Take::End],
        Take::Fail(cause) => vec![Take::Fail(cause)],
    })
}

/// Catch matching failures with a handler pull (`catchIf` with predicate
/// shape): a `Fail` satisfying `predicate` is replaced by driving
/// `handler(error)` to completion; non-matching failures pass through.
pub fn catch_if<A, E, P, F>(mut predicate: P, mut handler: F) -> Channel<A, E, A>
where
    A: Send + 'static,
    E: Send + 'static,
    P: FnMut(&E) -> bool + Send + 'static,
    F: FnMut(E) -> Pull<A, E> + Send + 'static,
{
    Channel::from_fn(move |take| match take {
        Take::Items(items) => vec![Take::Items(items)],
        Take::End => vec![Take::End],
        Take::Fail(cause) => match cause.into_failure() {
            Ok(error) if predicate(&error) => {
                let mut inner = handler(error);
                let mut out = Vec::new();
                loop {
                    match inner.pull() {
                        Take::Items(mut chunk) => out.append(&mut chunk),
                        Take::End => break,
                        Take::Fail(cause) => return vec![Take::Fail(cause)],
                    }
                }
                vec![Take::Items(out)]
            }
            Ok(error) => vec![Take::fail(error)],
            Err(cause) => vec![Take::Fail(cause)],
        },
    })
}

/// Catch every failure (`catchCauseFilter` with an error extractor): the
/// channel-level `Fail` already carries the typed error, so any failure is
/// replaced by driving `handler(error)` to completion.
pub fn catch_all<A, E, F>(handler: F) -> Channel<A, E, A>
where
    A: Send + 'static,
    E: Send + 'static,
    F: FnMut(E) -> Pull<A, E> + Send + 'static,
{
    catch_if(|_| true, handler)
}

/// Catch a cause the selector extracts (`catchCauseFilter`): the selector
/// reads the whole `Cause` and answers something to recover with, or nothing.
/// A `Fail` it answers for is replaced by driving `handler(extracted)` to
/// completion, and anything it declines passes through whole.
pub fn catch_cause_filter<A, E, X, S, F>(mut selector: S, mut handler: F) -> Channel<A, E, A>
where
    A: Send + 'static,
    E: Send + 'static,
    X: Send + 'static,
    S: FnMut(&Cause<E>) -> Option<X> + Send + 'static,
    F: FnMut(X) -> Pull<A, E> + Send + 'static,
{
    Channel::from_fn(move |take| match take {
        Take::Items(items) => vec![Take::Items(items)],
        Take::End => vec![Take::End],
        Take::Fail(cause) => match selector(&cause) {
            Some(extracted) => {
                let mut inner = handler(extracted);
                let mut out = Vec::new();
                loop {
                    match inner.pull() {
                        Take::Items(mut chunk) => out.append(&mut chunk),
                        Take::End => break,
                        Take::Fail(cause) => return vec![Take::Fail(cause)],
                    }
                }
                vec![Take::Items(out)]
            }
            None => vec![Take::Fail(cause)],
        },
    })
}

/// Catch with a fallback (`catchTag`/`catchReason` orElse shape): a `Fail`
/// satisfying `predicate` is replaced by driving `on_match(error)` to
/// completion; any other failure is replaced by driving `or_else(error)`
/// instead of passing through. Tag dispatch lives in the predicate, so
/// nested reasons compose through ordinary closures.
pub fn catch_or_else<A, E, P, F, G>(
    mut predicate: P,
    mut on_match: F,
    mut or_else: G,
) -> Channel<A, E, A>
where
    A: Send + 'static,
    E: Send + 'static,
    P: FnMut(&E) -> bool + Send + 'static,
    F: FnMut(E) -> Pull<A, E> + Send + 'static,
    G: FnMut(E) -> Pull<A, E> + Send + 'static,
{
    Channel::from_fn(move |take| match take {
        Take::Items(items) => vec![Take::Items(items)],
        Take::End => vec![Take::End],
        Take::Fail(cause) => match cause.into_failure() {
            Ok(error) => {
                let mut inner = if predicate(&error) {
                    on_match(error)
                } else {
                    or_else(error)
                };
                let mut out = Vec::new();
                loop {
                    match inner.pull() {
                        Take::Items(mut chunk) => out.append(&mut chunk),
                        Take::End => break,
                        Take::Fail(cause) => return vec![Take::Fail(cause)],
                    }
                }
                vec![Take::Items(out)]
            }
            Err(cause) => vec![Take::Fail(cause)],
        },
    })
}

/// Unwrap a nested reason (`unwrapReason`): a `Fail` satisfying `predicate`
/// is replaced by `Fail(reason(error))`; other failures pass through.
pub fn unwrap_reason<A, E, P, F>(mut predicate: P, mut reason: F) -> Channel<A, E, A>
where
    A: Send + 'static,
    E: Send + 'static,
    P: FnMut(&E) -> bool + Send + 'static,
    F: FnMut(E) -> E + Send + 'static,
{
    Channel::from_fn(move |take| match take {
        Take::Items(items) => vec![Take::Items(items)],
        Take::End => vec![Take::End],
        Take::Fail(cause) => match cause.into_failure() {
            Ok(error) if predicate(&error) => vec![Take::fail(reason(error))],
            Ok(error) => vec![Take::fail(error)],
            Err(cause) => vec![Take::Fail(cause)],
        },
    })
}

/// Decode UTF-8 bytes split across pulls (`decodeText` shapes): incomplete
/// trailing sequences buffer until complete; a stream ending mid-character
/// or with invalid bytes fails. Empty prefixes emit nothing (the driver
/// keeps pulling). Strict, never lossy.
pub fn decode_text<E>() -> Channel<u8, E, String>
where
    E: From<std::string::FromUtf8Error> + Send + 'static,
{
    let mut buffer: Vec<u8> = Vec::new();
    Channel::from_fn(move |take| match take {
        Take::Items(mut bytes) => {
            buffer.append(&mut bytes);
            let valid_up_to = match std::str::from_utf8(&buffer) {
                Ok(_) => buffer.len(),
                Err(error) => error.valid_up_to(),
            };
            let text =
                String::from_utf8(buffer.drain(..valid_up_to).collect()).expect("valid prefix");
            if text.is_empty() {
                Vec::new()
            } else {
                vec![Take::Items(vec![text])]
            }
        }
        Take::End => {
            if buffer.is_empty() {
                vec![Take::End]
            } else {
                match String::from_utf8(std::mem::take(&mut buffer)) {
                    Ok(text) => vec![Take::Items(vec![text]), Take::End],
                    Err(error) => vec![Take::fail(error.into())],
                }
            }
        }
        Take::Fail(cause) => vec![Take::Fail(cause)],
    })
}

/// Keep matching items per pull.
pub fn filter<A, E, F>(mut f: F) -> Channel<A, E, A>
where
    A: Send + Clone + 'static,
    E: Send + 'static,
    F: FnMut(&A) -> bool + Send + 'static,
{
    Channel::from_fn(move |take| match take {
        Take::Items(items) => vec![Take::Items(items.into_iter().filter(|a| f(a)).collect())],
        Take::End => vec![Take::End],
        Take::Fail(cause) => vec![Take::Fail(cause)],
    })
}

/// Which side's halt stops a [`merge`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HaltStrategy {
    /// Stop when the left side halts.
    Left,
    /// Stop when the right side halts.
    Right,
    /// Stop when either side halts.
    Either,
    /// Stop when both sides halted.
    Both,
}

/// Merge two pulls, interleaving takes as they arrive. A `Fail` always wins
/// immediately, whichever side it comes from; `End` handling follows the
/// halt strategy. Halted sides stop being pulled.
pub fn merge<A, E>(left: Pull<A, E>, right: Pull<A, E>, strategy: HaltStrategy) -> Pull<A, E>
where
    A: Send + 'static,
    E: Send + 'static,
{
    enum Side<A, E> {
        Left(Take<A, E>),
        Right(Take<A, E>),
    }
    // Keep the sides' own wake hooks: the consumer that stops early calls
    // them, so a producer blocked inside its pull can leave. Do not attach
    // them to the sides as halt hooks: a side's normal completion would then
    // drop the pull, fire the hook, and halt the other side mid-stream.
    let left_hooks = left.halt_hooks();
    let right_hooks = right.halt_hooks();
    let (tx, rx) = std::sync::mpsc::channel::<Side<A, E>>();
    let halted = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    for (pull, is_left) in [
        (std::sync::Arc::new(std::sync::Mutex::new(left)), true),
        (std::sync::Arc::new(std::sync::Mutex::new(right)), false),
    ] {
        let (tx, halted) = (tx.clone(), std::sync::Arc::clone(&halted));
        std::thread::spawn(move || {
            loop {
                if halted.load(std::sync::atomic::Ordering::SeqCst) {
                    break;
                }
                let take = pull.lock().unwrap().pull();
                let done = matches!(take, Take::End | Take::Fail(_));
                let side = if is_left {
                    Side::Left(take)
                } else {
                    Side::Right(take)
                };
                if tx.send(side).is_err() {
                    break;
                }
                if done {
                    break;
                }
            }
        });
    }
    drop(tx);
    let mut ended = false;
    let mut left_open = true;
    let mut right_open = true;
    let consumer_halt = std::sync::Arc::clone(&halted);
    let mut out = Pull::from_fn(move || {
        if ended {
            return Take::End;
        }
        loop {
            let side = match rx.recv() {
                Ok(side) => side,
                Err(_) => {
                    ended = true;
                    halted.store(true, std::sync::atomic::Ordering::SeqCst);
                    return Take::End;
                }
            };
            let (is_left, take) = match side {
                Side::Left(take) => (true, take),
                Side::Right(take) => (false, take),
            };
            match take {
                Take::Fail(cause) => {
                    ended = true;
                    halted.store(true, std::sync::atomic::Ordering::SeqCst);
                    return Take::Fail(cause);
                }
                Take::Items(items) => return Take::Items(items),
                Take::End => {
                    if is_left {
                        left_open = false;
                    } else {
                        right_open = false;
                    }
                    let halt = match strategy {
                        HaltStrategy::Either => true,
                        HaltStrategy::Left => !left_open,
                        HaltStrategy::Right => !right_open,
                        HaltStrategy::Both => !left_open && !right_open,
                    };
                    if halt {
                        ended = true;
                        halted.store(true, std::sync::atomic::Ordering::SeqCst);
                        return Take::End;
                    }
                }
            }
        }
    });
    for hook in left_hooks.into_iter().chain(right_hooks) {
        out = out.on_halt(move || hook());
    }
    out.on_halt(move || {
        consumer_halt.store(true, std::sync::atomic::Ordering::SeqCst);
    })
}

/// Merge two pulls with a deadline on every take: when no side answers in
/// time, producers are halted and the merged pull fails with the caller's
/// error (`timeoutFail` semantics) instead of hanging or silently
/// truncating. Fast merges behave exactly like [`merge`].
pub fn merge_with_deadline<A, E>(
    left: Pull<A, E>,
    right: Pull<A, E>,
    strategy: HaltStrategy,
    timeout: std::time::Duration,
    on_timeout: impl FnOnce() -> E + Send + 'static,
) -> Pull<A, E>
where
    A: Send + 'static,
    E: Send + 'static,
{
    enum Side<A, E> {
        Left(Take<A, E>),
        Right(Take<A, E>),
    }
    // Keep the sides' own wake hooks: the consumer that stops early calls
    // them, so a producer blocked inside its pull can leave. Do not attach
    // them to the sides as halt hooks: a side's normal completion would then
    // drop the pull, fire the hook, and halt the other side mid-stream.
    let left_hooks = left.halt_hooks();
    let right_hooks = right.halt_hooks();
    let (tx, rx) = std::sync::mpsc::channel::<Side<A, E>>();
    let halted = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    for (pull, is_left) in [
        (std::sync::Arc::new(std::sync::Mutex::new(left)), true),
        (std::sync::Arc::new(std::sync::Mutex::new(right)), false),
    ] {
        let (tx, halted) = (tx.clone(), std::sync::Arc::clone(&halted));
        std::thread::spawn(move || {
            loop {
                if halted.load(std::sync::atomic::Ordering::SeqCst) {
                    break;
                }
                let take = pull.lock().unwrap().pull();
                let done = matches!(take, Take::End | Take::Fail(_));
                let side = if is_left {
                    Side::Left(take)
                } else {
                    Side::Right(take)
                };
                if tx.send(side).is_err() {
                    break;
                }
                if done {
                    break;
                }
            }
        });
    }
    drop(tx);
    let mut ended = false;
    let mut left_open = true;
    let mut right_open = true;
    let mut on_timeout = Some(on_timeout);
    let consumer_halt = std::sync::Arc::clone(&halted);
    let mut out = Pull::from_fn(move || {
        if ended {
            return Take::End;
        }
        loop {
            let side = match rx.recv_timeout(timeout) {
                Ok(side) => side,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    ended = true;
                    halted.store(true, std::sync::atomic::Ordering::SeqCst);
                    let make = on_timeout.take().expect("timeout fires once");
                    return Take::fail(make());
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    ended = true;
                    halted.store(true, std::sync::atomic::Ordering::SeqCst);
                    return Take::End;
                }
            };
            let (is_left, take) = match side {
                Side::Left(take) => (true, take),
                Side::Right(take) => (false, take),
            };
            match take {
                Take::Fail(cause) => {
                    ended = true;
                    halted.store(true, std::sync::atomic::Ordering::SeqCst);
                    return Take::Fail(cause);
                }
                Take::Items(items) => return Take::Items(items),
                Take::End => {
                    if is_left {
                        left_open = false;
                    } else {
                        right_open = false;
                    }
                    let halt = match strategy {
                        HaltStrategy::Either => true,
                        HaltStrategy::Left => !left_open,
                        HaltStrategy::Right => !right_open,
                        HaltStrategy::Both => !left_open && !right_open,
                    };
                    if halt {
                        ended = true;
                        halted.store(true, std::sync::atomic::Ordering::SeqCst);
                        return Take::End;
                    }
                }
            }
        }
    });
    for hook in left_hooks.into_iter().chain(right_hooks) {
        out = out.on_halt(move || hook());
    }
    out.on_halt(move || {
        consumer_halt.store(true, std::sync::atomic::Ordering::SeqCst);
    })
}

/// Catch a defect (`catchDefect`): a take that failed with a defect is
/// replaced by driving `handler(defect)` to completion. A typed failure is not
/// a defect and passes through untouched.
pub fn catch_defect<A, E, F>(mut handler: F) -> Channel<A, E, A>
where
    A: Send + 'static,
    E: Send + 'static,
    F: FnMut(Defect) -> Pull<A, E> + Send + 'static,
{
    Channel::from_fn(move |take| match take {
        Take::Fail(cause) => match cause.into_defect() {
            Ok(defect) => {
                let mut inner = handler(defect);
                let mut out = Vec::new();
                loop {
                    match inner.pull() {
                        Take::Items(mut chunk) => out.append(&mut chunk),
                        Take::End => break,
                        Take::Fail(cause) => return vec![Take::Fail(cause)],
                    }
                }
                vec![Take::Items(out)]
            }
            Err(cause) => vec![Take::Fail(cause)],
        },
        other => vec![other],
    })
}
