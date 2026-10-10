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

/// Take the first `n` items, then end (`take` shapes).
pub fn take<A, E>(n: usize) -> Sink<A, E, Vec<A>>
where
    A: Send + Clone + 'static,
    E: Send + 'static,
{
    Sink::from_fn(move |mut source| {
        let mut out = Vec::new();
        while out.len() < n {
            match source.pull() {
                Take::Items(items) => {
                    let room = n - out.len();
                    out.extend(items.into_iter().take(room));
                }
                Take::End => break,
                Take::Fail(e) => return elm_effect::Exit::Failure(elm_effect::Cause::fail(e)),
            }
        }
        elm_effect::Exit::Success(out)
    })
}

/// Discard every item, succeeding with `()` (`drain`, `runDrain` shape).
pub fn drain<A, E>() -> Sink<A, E, ()>
where
    A: Send + 'static,
    E: Send + 'static,
{
    Sink::from_fn(move |mut source| {
        loop {
            match source.pull() {
                Take::Items(_) => continue,
                Take::End => return elm_effect::Exit::Success(()),
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

/// Take while a fallible predicate holds. `Err` stops with that failure.
pub fn take_while_effect<A, E>(
    mut pred: impl FnMut(&A) -> Result<bool, E> + Send + 'static,
) -> Sink<A, E, (Vec<A>, Vec<A>)>
where
    A: Send + 'static,
    E: Send + 'static,
{
    Sink::from_fn(move |mut source| {
        use elm_effect::Exit;
        let mut taken = Vec::new();
        loop {
            match source.pull() {
                Take::Items(items) => {
                    let mut stopped = false;
                    for item in items {
                        match pred(&item) {
                            Ok(true) => taken.push(item),
                            Ok(false) => {
                                stopped = true;
                                break;
                            }
                            Err(e) => return Exit::Failure(elm_effect::Cause::fail(e)),
                        }
                    }
                    if stopped {
                        break;
                    }
                }
                Take::End => break,
                Take::Fail(e) => return Exit::Failure(elm_effect::Cause::fail(e)),
            }
        }
        let mut leftover = Vec::new();
        loop {
            match source.pull() {
                Take::Items(items) => leftover.extend(items),
                Take::End => break,
                Take::Fail(e) => return Exit::Failure(elm_effect::Cause::fail(e)),
            }
        }
        Exit::Success((taken, leftover))
    })
}

/// Take while the projection yields `Some`; `None` stops and is consumed.
pub fn take_while_filter<A, B, E>(
    mut pred: impl FnMut(&A) -> Option<B> + Send + 'static,
) -> Sink<A, E, (Vec<B>, Vec<A>)>
where
    A: Send + 'static,
    B: Send + 'static,
    E: Send + 'static,
{
    Sink::from_fn(move |mut source| {
        use elm_effect::Exit;
        let mut taken = Vec::new();
        loop {
            match source.pull() {
                Take::Items(items) => {
                    let mut stopped = false;
                    for item in items {
                        match pred(&item) {
                            Some(mapped) => taken.push(mapped),
                            None => {
                                stopped = true;
                                break;
                            }
                        }
                    }
                    if stopped {
                        break;
                    }
                }
                Take::End => break,
                Take::Fail(e) => return Exit::Failure(elm_effect::Cause::fail(e)),
            }
        }
        let mut leftover = Vec::new();
        loop {
            match source.pull() {
                Take::Items(items) => leftover.extend(items),
                Take::End => break,
                Take::Fail(e) => return Exit::Failure(elm_effect::Cause::fail(e)),
            }
        }
        Exit::Success((taken, leftover))
    })
}

/// A platform sink: write byte chunks to any [`Write`]r. `End` flushes;
/// `Fail` aborts without flushing and propagates the failure.
pub fn from_write<W, E>(mut writer: W) -> Sink<u8, E, ()>
where
    W: std::io::Write + Send + 'static,
    E: Send + 'static,
{
    Sink::from_fn(move |mut source| {
        use elm_effect::Exit;
        loop {
            match source.pull() {
                Take::Items(items) => {
                    if writer.write_all(&items).is_err() {
                        return Exit::Failure(elm_effect::Cause::die(elm_effect::Defect::new(
                            "sink writer failed",
                        )));
                    }
                }
                Take::End => {
                    let _ = writer.flush();
                    return Exit::Success(());
                }
                Take::Fail(e) => return Exit::Failure(elm_effect::Cause::fail(e)),
            }
        }
    })
}

/// [`from_write`] with an interruption probe: an aborted copy must not look
/// like a close. `End` still flushes; once `interrupted` reports true the
/// sink stops without flushing and exits with an interrupt cause, so bytes
/// written so far survive but no close-side flush runs.
pub fn from_write_interruptible<W, E>(
    mut writer: W,
    interrupted: impl Fn() -> bool + Send + 'static,
) -> Sink<u8, E, ()>
where
    W: std::io::Write + Send + 'static,
    E: Send + 'static,
{
    Sink::from_fn(move |mut source| {
        use elm_effect::Exit;
        loop {
            if interrupted() {
                return Exit::Failure(elm_effect::Cause::interrupt(None));
            }
            match source.pull() {
                Take::Items(items) => {
                    if writer.write_all(&items).is_err() {
                        return Exit::Failure(elm_effect::Cause::die(elm_effect::Defect::new(
                            "sink writer failed",
                        )));
                    }
                }
                Take::End => {
                    let _ = writer.flush();
                    return Exit::Success(());
                }
                Take::Fail(e) => return Exit::Failure(elm_effect::Cause::fail(e)),
            }
        }
    })
}
