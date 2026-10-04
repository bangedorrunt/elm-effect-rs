//! Law L13 (fan-out part): `all` runs one body at a time unless bounded.
//!
//! Bodies are fiber closures: a synchronous `Effect` cannot suspend, so an
//! "is suspended" test over effects would be vacuous. Every body runs as a
//! fiber (supervisor, finalizers, interruption); the combinator only decides
//! how many run at once. Fail-fast: the first failure interrupts the rest
//! and wins; `validate` (.26) is the collecting twin.

use crate::{Cause, Ctx, Exit, FiberHandle, ForkOpts, Reason, Runtime};

/// How many bodies run at once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Concurrency {
    /// One at a time, in order. The default.
    Sequential,
    /// At most `n` at once (`n == 0` behaves as sequential).
    Bounded(usize),
    /// Every body at once. `policy::remote` never selects this.
    Unbounded,
}

/// A fiber body: anything a fiber can run.
pub type Body<A, E> = Box<dyn FnOnce(&Ctx) -> Exit<A, E> + Send>;

/// Run every body, collecting successes in order. The first failure
/// interrupts every other body (released before this returns) and wins.
pub fn all<A, E>(rt: &Runtime, bodies: Vec<Body<A, E>>, concurrency: Concurrency) -> Exit<Vec<A>, E>
where
    A: Send + 'static,
    E: Send + 'static,
{
    match concurrency {
        Concurrency::Sequential | Concurrency::Bounded(0) => all_sequential(rt, bodies),
        Concurrency::Bounded(n) => all_bounded(rt, bodies, n),
        Concurrency::Unbounded => all_bounded(rt, bodies, usize::MAX),
    }
}

fn all_sequential<A, E>(rt: &Runtime, bodies: Vec<Body<A, E>>) -> Exit<Vec<A>, E>
where
    A: Send + 'static,
    E: Send + 'static,
{
    let mut out = Vec::with_capacity(bodies.len());
    for body in bodies {
        match rt.fork_detach_with(body, ForkOpts::immediate()).join() {
            Exit::Success(a) => out.push(a),
            Exit::Failure(cause) => return Exit::Failure(cause),
        }
    }
    Exit::Success(out)
}

fn all_bounded<A, E>(rt: &Runtime, bodies: Vec<Body<A, E>>, bound: usize) -> Exit<Vec<A>, E>
where
    A: Send + 'static,
    E: Send + 'static,
{
    let total = bodies.len();
    if total == 0 {
        return Exit::Success(Vec::new());
    }
    // Lazily spawned: at most `bound` fibers are ever alive, so the peak
    // in flight respects the bound by construction (no semaphore needed).
    let bound = bound.min(total).max(1);
    enum Done<A, E> {
        Ok(usize, A),
        Failed(usize, Cause<E>),
    }
    let (tx, rx) = std::sync::mpsc::channel::<Done<A, E>>();
    let mut pending = bodies.into_iter().enumerate();
    // Live fibers by index, for interrupting the losers on failure.
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
                    let _ = tx.send(match exit {
                        Exit::Success(a) => Done::Ok(index, a),
                        Exit::Failure(cause) => Done::Failed(index, cause),
                    });
                    Exit::<(), std::convert::Infallible>::succeed(())
                },
                ForkOpts::immediate(),
            );
            live.insert(index, handle);
        }
    };
    for _ in 0..bound {
        spawn_one(&mut pending, &mut live);
    }
    let mut results: Vec<Option<A>> = (0..total).map(|_| None).collect();
    let mut finished = 0;
    let mut failure: Option<Cause<E>> = None;
    while finished < total {
        match rx.recv().expect("a body always answers") {
            Done::Ok(index, a) => {
                live.remove(&index);
                if failure.is_none() {
                    results[index] = Some(a);
                    spawn_one(&mut pending, &mut live);
                }
                finished += 1;
            }
            Done::Failed(index, cause) => {
                live.remove(&index);
                if failure.is_none() {
                    failure = Some(cause);
                    // Fail fast: interrupt every live sibling. Their exits
                    // still arrive below and are drained, not recorded.
                    for handle in live.values() {
                        handle.interrupt_unsafe();
                    }
                }
                finished += 1;
            }
        }
    }
    // Reap every fiber so no task outlives the call.
    for handle in live.into_values() {
        handle.join();
    }
    match failure {
        Some(cause) => Exit::Failure(cause),
        None => Exit::Success(results.into_iter().map(|o| o.expect("all ok")).collect()),
    }
}

/// Run `f` over every item with `all`'s bounds, collecting in order.
pub fn for_each<T, A, E>(
    rt: &Runtime,
    items: Vec<T>,
    concurrency: Concurrency,
    f: impl Fn(T, &Ctx) -> Exit<A, E> + Send + Sync + 'static,
) -> Exit<Vec<A>, E>
where
    T: Send + 'static,
    A: Send + 'static,
    E: Send + 'static,
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
    all(rt, bodies, concurrency)
}

/// Run every body, collecting successes and typed failures in order. A
/// defect or interruption fails fast: live siblings are interrupted and
/// the cause wins. Single `Fail` causes land in the failure list.
pub fn partition<A, E>(
    rt: &Runtime,
    bodies: Vec<Body<A, E>>,
    concurrency: Concurrency,
) -> Exit<(Vec<A>, Vec<E>), E>
where
    A: Send + 'static,
    E: Send + 'static,
{
    match concurrency {
        Concurrency::Sequential => partition_sequential(rt, bodies),
        Concurrency::Bounded(n) => partition_bounded(rt, bodies, n),
        Concurrency::Unbounded => partition_bounded(rt, bodies, usize::MAX),
    }
}

/// Like [`partition`], but any typed failure fails the whole run with every
/// collected error. Defects and interrupts still fail fast on their own.
pub fn validate<A, E>(
    rt: &Runtime,
    bodies: Vec<Body<A, E>>,
    concurrency: Concurrency,
) -> Exit<Vec<A>, Vec<E>>
where
    A: Send + 'static,
    E: Send + 'static,
{
    match partition(rt, bodies, concurrency) {
        Exit::Success((passes, fails)) => {
            if fails.is_empty() {
                Exit::Success(passes)
            } else {
                Exit::Failure(Cause::fail(fails))
            }
        }
        Exit::Failure(cause) => Exit::Failure(map_cause(cause)),
    }
}

/// Like [`validate`], discarding successes: returns `()` when everything
/// passes, every collected error otherwise.
pub fn validate_discard<A, E>(
    rt: &Runtime,
    bodies: Vec<Body<A, E>>,
    concurrency: Concurrency,
) -> Exit<(), Vec<E>>
where
    A: Send + 'static,
    E: Send + 'static,
{
    match validate(rt, bodies, concurrency) {
        Exit::Success(_) => Exit::Success(()),
        Exit::Failure(cause) => Exit::Failure(cause),
    }
}

fn partition_sequential<A, E>(rt: &Runtime, bodies: Vec<Body<A, E>>) -> Exit<(Vec<A>, Vec<E>), E>
where
    A: Send + 'static,
    E: Send + 'static,
{
    let mut passes = Vec::with_capacity(bodies.len());
    let mut fails = Vec::new();
    for body in bodies {
        match rt.fork_detach_with(body, ForkOpts::immediate()).join() {
            Exit::Success(a) => passes.push(a),
            Exit::Failure(cause) => match only_fail(cause) {
                Ok(e) => fails.push(e),
                Err(cause) => return Exit::Failure(cause),
            },
        }
    }
    Exit::Success((passes, fails))
}

fn partition_bounded<A, E>(
    rt: &Runtime,
    bodies: Vec<Body<A, E>>,
    bound: usize,
) -> Exit<(Vec<A>, Vec<E>), E>
where
    A: Send + 'static,
    E: Send + 'static,
{
    let total = bodies.len();
    if total == 0 {
        return Exit::Success((Vec::new(), Vec::new()));
    }
    // Lazily spawned: at most `bound` fibers are ever alive, so the peak
    // in flight respects the bound by construction (no semaphore needed).
    let bound = bound.min(total).max(1);
    enum Done<A, E> {
        Ok(usize, A),
        Failed(usize, Cause<E>),
    }
    let (tx, rx) = std::sync::mpsc::channel::<Done<A, E>>();
    let mut pending = bodies.into_iter().enumerate();
    // Live fibers by index, for interrupting the losers on failure.
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
                    let _ = tx.send(match exit {
                        Exit::Success(a) => Done::Ok(index, a),
                        Exit::Failure(cause) => Done::Failed(index, cause),
                    });
                    Exit::<(), std::convert::Infallible>::succeed(())
                },
                ForkOpts::immediate(),
            );
            live.insert(index, handle);
        }
    };
    for _ in 0..bound {
        spawn_one(&mut pending, &mut live);
    }
    let mut passes: Vec<Option<A>> = (0..total).map(|_| None).collect();
    let mut fails: Vec<Option<E>> = (0..total).map(|_| None).collect();
    let mut finished = 0;
    let mut failure: Option<Cause<E>> = None;
    while finished < total {
        match rx.recv().expect("a body always answers") {
            Done::Ok(index, a) => {
                live.remove(&index);
                if failure.is_none() {
                    passes[index] = Some(a);
                    spawn_one(&mut pending, &mut live);
                }
                finished += 1;
            }
            Done::Failed(index, cause) => {
                live.remove(&index);
                if failure.is_none() {
                    match only_fail(cause) {
                        Ok(e) => {
                            fails[index] = Some(e);
                            spawn_one(&mut pending, &mut live);
                        }
                        Err(cause) => {
                            failure = Some(cause);
                            // Fail fast: interrupt every live sibling. Their exits
                            // still arrive below and are drained, not recorded.
                            for handle in live.values() {
                                handle.interrupt_unsafe();
                            }
                        }
                    }
                }
                finished += 1;
            }
        }
    }
    match failure {
        Some(cause) => Exit::Failure(cause),
        None => Exit::Success((
            passes.into_iter().flatten().collect(),
            fails.into_iter().flatten().collect(),
        )),
    }
}

/// The typed error if the cause is exactly one `Fail`; otherwise the cause
/// back for fail-fast handling. (Mirrors the schedule driver; kept local so
/// this module stands alone.)
fn only_fail<E>(cause: Cause<E>) -> Result<E, Cause<E>> {
    match cause.reasons.as_slice() {
        [Reason::Fail(_)] => {
            let mut reasons = cause.reasons;
            match reasons.pop() {
                Some(Reason::Fail(e)) => Ok(e),
                _ => unreachable!("matched above"),
            }
        }
        _ => Err(cause),
    }
}

/// Rewrap a call failure: typed errors become `Inner`-style pass-through
/// here (identity on reasons), used to change the error channel.
fn map_cause<E>(cause: Cause<E>) -> Cause<Vec<E>> {
    let mut out = Cause::empty();
    for reason in cause.reasons {
        out.reasons.push(match reason {
            Reason::Fail(e) => Reason::Fail(vec![e]),
            Reason::Die(d) => Reason::Die(d),
            Reason::Interrupt { fiber_id } => Reason::Interrupt { fiber_id },
        });
    }
    out
}
