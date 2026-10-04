//! `Sink.ts` port: fold takes to a value. Only sinks interpret pulls —
//! everything upstream stays lazy. Sinks run on closed pulls here; providing
//! services happens before sinking, as with `run_fold`.

use super::pull::Pull;
use super::take::Take;

/// A consumer driver: pulls takes until done.
pub type FoldFn<A, E, S> = Box<dyn FnMut(Pull<A, E>) -> elm_effect::Exit<S, E> + Send>;

/// A consumer: folds takes into one value.
pub struct Sink<A, E, S> {
    fold: FoldFn<A, E, S>,
}

impl<A, E, S> Sink<A, E, S>
where
    A: Send + 'static,
    E: Send + 'static,
    S: Send + 'static,
{
    /// Build from a driver.
    pub fn from_fn(
        fold: impl FnMut(Pull<A, E>) -> elm_effect::Exit<S, E> + Send + 'static,
    ) -> Self {
        Self {
            fold: Box::new(fold),
        }
    }

    /// Run to completion: `End` finishes, `Fail` propagates.
    pub fn run(mut self, source: Pull<A, E>) -> elm_effect::Exit<S, E> {
        (self.fold)(source)
    }
}

/// Fold every item into state.
pub fn fold<A, E, S>(init: S, mut f: impl FnMut(S, Vec<A>) -> S + Send + 'static) -> Sink<A, E, S>
where
    A: Send + 'static,
    E: Send + 'static,
    S: Send + Clone + 'static,
{
    Sink::from_fn(move |mut source| {
        let mut state = init.clone();
        loop {
            match source.pull() {
                Take::Items(items) => state = f(state, items),
                Take::End => return elm_effect::Exit::Success(state),
                Take::Fail(e) => return elm_effect::Exit::Failure(elm_effect::Cause::fail(e)),
            }
        }
    })
}

/// Collect every item.
pub fn collect<A, E>() -> Sink<A, E, Vec<A>>
where
    A: Send + 'static,
    E: Send + 'static,
{
    Sink::from_fn(move |mut source| {
        let mut out = Vec::new();
        loop {
            match source.pull() {
                Take::Items(items) => out.extend(items),
                Take::End => return elm_effect::Exit::Success(out),
                Take::Fail(e) => return elm_effect::Exit::Failure(elm_effect::Cause::fail(e)),
            }
        }
    })
}

/// Count items.
pub fn count<A, E>() -> Sink<A, E, usize>
where
    A: Send + 'static,
    E: Send + 'static,
{
    Sink::from_fn(move |mut source| {
        let mut n = 0usize;
        loop {
            match source.pull() {
                Take::Items(items) => n += items.len(),
                Take::End => return elm_effect::Exit::Success(n),
                Take::Fail(e) => return elm_effect::Exit::Failure(elm_effect::Cause::fail(e)),
            }
        }
    })
}

/// First item, if any.
pub fn head<A, E>() -> Sink<A, E, Option<A>>
where
    A: Send + 'static,
    E: Send + 'static,
{
    Sink::from_fn(move |mut source| {
        loop {
            match source.pull() {
                Take::Items(mut items) if !items.is_empty() => {
                    return elm_effect::Exit::Success(Some(items.remove(0)));
                }
                Take::Items(_) => continue,
                Take::End => return elm_effect::Exit::Success(None),
                Take::Fail(e) => return elm_effect::Exit::Failure(elm_effect::Cause::fail(e)),
            }
        }
    })
}

/// Take while the predicate holds, then end (`takeWhileFilter` shapes).
pub fn take_while<A, E>(
    mut predicate: impl FnMut(&A) -> bool + Send + 'static,
) -> Sink<A, E, Vec<A>>
where
    A: Send + Clone + 'static,
    E: Send + 'static,
{
    Sink::from_fn(move |mut source| {
        let mut out = Vec::new();
        loop {
            match source.pull() {
                Take::Items(items) => {
                    let mut stopped = false;
                    for item in items {
                        if !stopped && predicate(&item) {
                            out.push(item);
                        } else {
                            stopped = true;
                        }
                    }
                    if stopped {
                        return elm_effect::Exit::Success(out);
                    }
                }
                Take::End => return elm_effect::Exit::Success(out),
                Take::Fail(e) => return elm_effect::Exit::Failure(elm_effect::Cause::fail(e)),
            }
        }
    })
}
