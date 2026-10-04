//! Law L13 (fan-out part): `all` runs one body at a time unless bounded.
//!
//! Bodies are fiber closures: a synchronous `Effect` cannot suspend, so an
//! "is suspended" test over effects would be vacuous. Every body runs as a
//! fiber (supervisor, finalizers, interruption); the combinator only decides
//! how many run at once. Fail-fast: the first failure interrupts the rest
//! and wins; `validate` (.26) is the collecting twin.

use elm_effect::{Cause, Ctx, Exit, FiberHandle, ForkOpts, Runtime};

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
