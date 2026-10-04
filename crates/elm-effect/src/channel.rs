//! `Channel.ts` port: pull-to-pull transforms, the stream conversation's
//! middle. A channel maps each take and passes `End`/`Fail` through, so
//! termination and failure need no special handling downstream.

use super::pull::Pull;
use super::take::Take;

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

    /// Drive `source` through this channel, yielding transformed takes.
    /// Every take (items, end, failure) flows through `step`: termination
    /// and failure handling belong to the channel, not the driver. If a
    /// terminal take produces nothing, the stream ends.
    pub fn run(self, source: Pull<A, E>) -> Pull<B, E> {
        let mut source = source;
        let mut step = self.step;
        let mut pending: Vec<Take<B, E>> = Vec::new();
        Pull::from_fn(move || {
            loop {
                if !pending.is_empty() {
                    return pending.remove(0);
                }
                let take = source.pull();
                let terminal = !matches!(take, Take::Items(_));
                pending = step(take);
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
        Take::Fail(e) => vec![Take::Fail(e)],
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
        Take::Fail(e) if predicate(&e) => vec![Take::End],
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
                    Err(e) => return vec![Take::Fail(e)],
                }
            }
            vec![Take::Items(out)]
        }
        Take::End => vec![Take::End],
        Take::Fail(e) => vec![Take::Fail(e)],
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
                        Take::Fail(e) => return vec![Take::Fail(e)],
                    }
                }
            }
            vec![Take::Items(out)]
        }
        Take::End => vec![Take::End],
        Take::Fail(e) => vec![Take::Fail(e)],
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
        Take::Fail(e) => vec![Take::Fail(e)],
    })
}
