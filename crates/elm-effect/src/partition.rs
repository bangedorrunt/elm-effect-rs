//! Phase 4 collect combinators, read from the pin before encoding
//! (`packages/effect/src/Effect.ts`, `partition` + `validate` sections).
//!
//! Pin semantics (the v4 guides do not spell these, the source does):
//! - `partition` runs EVERY element and never fails: `([passes], [fails])`.
//! - `validate` evaluates EVERY element; any failure discards the successes
//!   and returns ALL failures, else all successes.
//! Both differ from `all` (.24), which fails fast and interrupts siblings:
//! here siblings are never interrupted.
//!
//! Bodies are fiber closures (a synchronous `Effect` cannot suspend).
//! `Concurrency` mirrors the `.24` bound type on purpose (same three
//! shapes); the two definitions unify when `lib.rs` wires both modules.

use elm_effect::{Cause, Ctx, Defect, Exit, FiberHandle, ForkOpts, Reason, Runtime};

/// How many bodies run at once. Mirrors `.24` `Concurrency` exactly.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Concurrency {
    /// One at a time, in order. The default.
    Sequential,
    /// At most `n` at once (`n == 0` behaves as sequential).
    Bounded(usize),
    /// Every body at once.
    Unbounded,
}

/// A fiber body: anything a fiber can run.
pub type Body<A, E> = Box<dyn FnOnce(&Ctx) -> Exit<A, E> + Send>;

/// Split a failure into a collectable typed error or a propagating defect.
/// Mirrors the pin: `partition`/`validate` run `result` per element, so a
/// clean single `Fail` collects while `Die`/`Interrupt` still kill the
/// whole combinator (rebuilt into the empty error channel).
fn narrow<E>(cause: Cause<E>) -> Result<E, Cause<std::convert::Infallible>>
where
    E: std::fmt::Debug,
{
    let mut reasons = cause.reasons;
    if reasons.len() == 1 {
        if let Reason::Fail(e) = reasons.pop().unwrap() {
            return Ok(e);
        }
    }
    let mut out = Cause::empty();
    for reason in reasons {
        out.reasons.push(match reason {
            Reason::Fail(e) => Reason::Die(Defect::new(format!("uncollectable failure: {e:?}"))),
            Reason::Die(d) => Reason::Die(d),
            Reason::Interrupt { fiber_id } => Reason::Interrupt { fiber_id },
        });
    }
    Err(out)
}

/// Run every body to completion, collecting indexed exits. Siblings are
/// never interrupted: failures are data here, not control flow.
fn run_all<A, E>(
    rt: &Runtime,
    bodies: Vec<Body<A, E>>,
    concurrency: Concurrency,
) -> Vec<(usize, Exit<A, E>)>
where
    A: Send + 'static,
    E: Send + 'static,
{
    let total = bodies.len();
    let bound = match concurrency {
        Concurrency::Sequential | Concurrency::Bounded(0) => 1,
        Concurrency::Bounded(n) => n.min(total).max(1),
        Concurrency::Unbounded => total.max(1),
    };
    let (tx, rx) = std::sync::mpsc::channel::<(usize, Exit<A, E>)>();
    let mut pending = bodies.into_iter().enumerate();
    let mut live: std::collections::HashMap<usize, FiberHandle<(), std::convert::Infallible>> =
        std::collections::HashMap::new();
    let spawn_one = |pending: &mut std::iter::Enumerate<std::vec::IntoIter<Body<A, E>>>,
                     live: &mut std::collections::HashMap<
        usize,
        FiberHandle<(), std::convert::Infallible>,
    >| {
        if let Some((index, body)) = pending.next() {
            let tx = tx.clone();
            let handle = rt.fork_detach_with(
                move |ctx| {
                    let exit = body(ctx);
                    let _ = tx.send((index, exit));
                    Exit::<(), std::convert::Infallible>::succeed(())
                },
                ForkOpts::immediate(),
            );
            live.insert(index, handle);
        }
    };
    let mut pending = pending;
    let mut live = live;
    for _ in 0..bound.min(total) {
        spawn_one(&mut pending, &mut live);
    }
    let mut out = Vec::with_capacity(total);
    while out.len() < total {
        let (index, exit) = rx.recv().expect("a body always answers");
        live.remove(&index);
        out.push((index, exit));
        spawn_one(&mut pending, &mut live);
    }
    for handle in live.into_values() {
        handle.join();
    }
    out
}

/// Run `f` over every item, separating successes from failures. Never fails:
/// every element is evaluated even after failures.
pub fn partition<T, A, E>(
    rt: &Runtime,
    items: Vec<T>,
    concurrency: Concurrency,
    f: impl Fn(T, &Ctx) -> Exit<A, E> + Send + Sync + 'static,
) -> Exit<(Vec<A>, Vec<E>), std::convert::Infallible>
where
    T: Send + 'static,
    A: Send + 'static,
    E: Send + 'static + std::fmt::Debug,
{
    use std::sync::Arc;
    let f = Arc::new(f);
    let bodies: Vec<Body<A, E>> = items
        .into_iter()
        .map(|item| {
            let f = Arc::clone(&f);
            Box::new(move |ctx: &Ctx| f(item, ctx)) as Body<A, E>
        })
        .collect();
    let mut outcomes = run_all(rt, bodies, concurrency);
    outcomes.sort_by_key(|(index, _)| *index);
    let mut passes = Vec::new();
    let mut fails = Vec::new();
    for (_, exit) in outcomes {
        match exit {
            Exit::Success(a) => passes.push(a),
            Exit::Failure(cause) => match narrow(cause) {
                Ok(e) => fails.push(e),
                Err(die) => return Exit::Failure(die),
            },
        }
    }
    Exit::Success((passes, fails))
}

/// Run `f` over every item, accumulating all failures. Every element is
/// evaluated; any failure discards the successes. Like the pin, this is
/// `partition` plus a fold: typed failures collect, while `Die`/`Interrupt`
/// still kill the whole combinator (fail-fast, as `forEach` does under the
/// pin's `result` wrapper).
pub fn validate<T, A, E>(
    rt: &Runtime,
    items: Vec<T>,
    concurrency: Concurrency,
    f: impl Fn(T, &Ctx) -> Exit<A, E> + Send + Sync + 'static,
) -> Exit<Vec<A>, Vec<E>>
where
    T: Send + 'static,
    A: Send + 'static,
    E: Send + 'static + std::fmt::Debug,
{
    use std::sync::Arc;
    let f = Arc::new(f);
    let bodies: Vec<Body<A, E>> = items
        .into_iter()
        .map(|item| {
            let f = Arc::clone(&f);
            Box::new(move |ctx: &Ctx| f(item, ctx)) as Body<A, E>
        })
        .collect();
    let mut outcomes = run_all(rt, bodies, concurrency);
    outcomes.sort_by_key(|(index, _)| *index);
    let mut values = Vec::new();
    let mut failures = Vec::new();
    for (_, exit) in outcomes {
        match exit {
            Exit::Success(a) => values.push(a),
            Exit::Failure(cause) => match narrow(cause) {
                Ok(e) => failures.push(e),
                // A defect kills validation outright (mirrors fiber death:
                // loud, never a silent collect). Unreachable in
                // typed-failure tests.
                Err(die) => panic!("validate hit a defect, which kills validation: {die:?}"),
            },
        }
    }
    if failures.is_empty() {
        Exit::Success(values)
    } else {
        Exit::Failure(Cause::fail(failures))
    }
}
