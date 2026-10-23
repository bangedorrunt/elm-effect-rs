//! Law L13 (fan-out part): `all` runs one body at a time unless bounded.
//!
//! Bodies are fiber closures: a synchronous `Effect` cannot suspend, so an
//! "is suspended" test over effects would be vacuous. Every body runs as a
//! fiber (supervisor, finalizers, interruption); the combinator only decides
//! how many run at once. Fail-fast: the first failure interrupts the rest
//! and wins; `validate` (.26) is the collecting twin.
// This file belongs to the parked port campaign: a test reaches it through
// a `#[path]` include to port specific rows, so most of its surface is
// unused by any one test target. Warnings about the unexercised remainder
// describe the test's coverage, not a defect in the crate, so they are
// allowed here rather than fixed one by one.
#![allow(dead_code)]
use std::sync::Arc;

use crate::{Cause, Ctx, Defect, Exit, FiberHandle, ForkOpts, Reason, Runtime};

/// Bound for one fan-out wait: a body that answers nothing for this long —
/// hung past every interrupt — fails the combinator with a defect naming the
/// missing indexes instead of hanging the caller forever. Slow-but-steady
/// pipelines keep delivering inside the bound, so only a stalled wait trips
/// it. Use the `_with_deadline` variants for a caller-supplied bound.
pub const BOUNDED_RECV_TIMEOUT: crate::Duration = crate::Duration::from_secs(30);

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
    all_with_deadline(rt, bodies, concurrency, BOUNDED_RECV_TIMEOUT)
}

/// Like [`all`] with a caller-supplied bound on each fan-out wait: a body
/// that answers nothing for `deadline` fails the run with a defect naming
/// the missing indexes. Live siblings are interrupted and detached — never
/// joined, since an unkillable body would hang the join too.
pub fn all_with_deadline<A, E>(
    rt: &Runtime,
    bodies: Vec<Body<A, E>>,
    concurrency: Concurrency,
    deadline: crate::Duration,
) -> Exit<Vec<A>, E>
where
    A: Send + 'static,
    E: Send + 'static,
{
    let std_deadline = std::time::Duration::from_millis(deadline.as_millis());
    match concurrency {
        Concurrency::Sequential | Concurrency::Bounded(0) => all_sequential(rt, bodies),
        Concurrency::Bounded(n) => all_bounded(rt, bodies, n, std_deadline),
        Concurrency::Unbounded => all_bounded(rt, bodies, usize::MAX, std_deadline),
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

fn all_bounded<A, E>(
    rt: &Runtime,
    bodies: Vec<Body<A, E>>,
    bound: usize,
    deadline: std::time::Duration,
) -> Exit<Vec<A>, E>
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
        match rx.recv_timeout(deadline) {
            Ok(Done::Ok(index, a)) => {
                // Join rather than drop: a body reports over the channel
                // before its finalizers run, so dropping the handle would let
                // the batch return while a release is still in flight. The
                // join is also where a cleanup that died becomes visible.
                let spilled = match live.remove(&index).map(|handle| handle.join()) {
                    Some(Exit::Failure(cause)) => {
                        cause.defects().into_iter().cloned().collect::<Vec<_>>()
                    }
                    _ => Vec::new(),
                };
                if !spilled.is_empty() && failure.is_none() {
                    failure = Some(Cause::from_defects(spilled));
                    for handle in live.values() {
                        handle.interrupt_unsafe();
                    }
                }
                if failure.is_none() {
                    results[index] = Some(a);
                    spawn_one(&mut pending, &mut live);
                }
                finished += 1;
            }
            Ok(Done::Failed(index, cause)) => {
                // Same join, same reason: the failing body's own cleanup runs
                // after it reported, and the batch must not return before it.
                let tail = live.remove(&index).map(|handle| handle.join());
                let mut cause = cause;
                if let Some(Exit::Failure(tail)) = tail {
                    cause.append_defects(tail.defects().into_iter().cloned().collect());
                }
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
            Err(_) => {
                // Stalled wait: name the missing bodies, interrupt the live,
                // and detach. No join: an unkillable body would hang it too.
                let missing: Vec<usize> = results
                    .iter()
                    .enumerate()
                    .filter(|(_, r)| r.is_none())
                    .map(|(i, _)| i)
                    .collect();
                for handle in live.values() {
                    handle.interrupt_unsafe();
                }
                return Exit::Failure(Cause::die(Defect::new(format!(
                    "bounded fan-out timed out after {deadline:?} waiting for bodies {missing:?}"
                ))));
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

/// `Effect.all(..., { discard: true })`: the same fan-out with its results
/// dropped. A failure is still the run's failure.
pub fn all_discard<A, E>(
    rt: &Runtime,
    bodies: Vec<Body<A, E>>,
    concurrency: Concurrency,
) -> Exit<(), E>
where
    A: Send + 'static,
    E: Send + 'static,
{
    match all(rt, bodies, concurrency) {
        Exit::Success(_) => Exit::Success(()),
        Exit::Failure(cause) => Exit::Failure(cause),
    }
}

/// Run every body and collect its exit, with at most `bound` alive at a time.
/// Unlike [`all_bounded`] nothing is interrupted and no exit is dropped.
fn tolerant_bounded<A, E>(rt: &Runtime, bodies: Vec<Body<A, E>>, bound: usize) -> Vec<Exit<A, E>>
where
    A: Send + 'static,
    E: Send + 'static,
{
    let mut exits = Vec::new();
    let mut pending = bodies.into_iter();
    loop {
        let batch: Vec<Body<A, E>> = pending.by_ref().take(bound.max(1)).collect();
        if batch.is_empty() {
            return exits;
        }
        let handles: Vec<_> = batch
            .into_iter()
            .map(|body| rt.fork_detach_with(body, ForkOpts::immediate()))
            .collect();
        for handle in handles {
            exits.push(handle.join());
        }
    }
}

/// `Effect.all(..., { mode: "result" })`: every body runs, and a body whose
/// cause is exactly its typed failure comes back as `Err` instead of failing
/// the run.
///
/// A defect or an interrupt is not the caller's typed error and cannot be
/// handed back as a value, so one of those still fails the run (the other
/// results are dropped, as the pin drops them).
pub fn all_result<A, E>(
    rt: &Runtime,
    bodies: Vec<Body<A, E>>,
    concurrency: Concurrency,
) -> Exit<Vec<Result<A, E>>, E>
where
    A: Send + 'static,
    E: Send + Clone + 'static,
{
    let bound = match concurrency {
        Concurrency::Sequential | Concurrency::Bounded(0) => 1,
        Concurrency::Bounded(n) => n,
        Concurrency::Unbounded => bodies.len().max(1),
    };
    let mut results = Vec::with_capacity(bodies.len());
    let mut hard: Option<Cause<E>> = None;
    for exit in tolerant_bounded(rt, bodies, bound) {
        match exit {
            Exit::Success(value) => results.push(Some(Ok(value))),
            Exit::Failure(cause) => {
                let typed_only = cause.has_fails() && !cause.has_dies() && !cause.has_interrupts();
                match cause.failure().filter(|_| typed_only) {
                    Some(error) => results.push(Some(Err(error.clone()))),
                    None => {
                        hard = hard.or(Some(cause));
                        results.push(None);
                    }
                }
            }
        }
    }
    match hard {
        Some(cause) => Exit::Failure(cause),
        None => Exit::Success(
            results
                .into_iter()
                .map(|result| result.expect("every body was captured"))
                .collect(),
        ),
    }
}

/// `Effect.all(record, { mode: "result" })`: the same as [`all_result`] with
/// the record's keys carried alongside their results, in insertion order.
pub fn all_result_record<A, E>(
    rt: &Runtime,
    entries: Vec<(String, Body<A, E>)>,
    concurrency: Concurrency,
) -> Exit<Vec<(String, Result<A, E>)>, E>
where
    A: Send + 'static,
    E: Send + Clone + 'static,
{
    let keys: Vec<String> = entries.iter().map(|(key, _)| key.clone()).collect();
    let bodies: Vec<Body<A, E>> = entries.into_iter().map(|(_, body)| body).collect();
    match all_result(rt, bodies, concurrency) {
        Exit::Success(results) => Exit::Success(keys.into_iter().zip(results).collect()),
        Exit::Failure(cause) => Exit::Failure(cause),
    }
}

/// `Effect.head`: the first element of a collection, or `on_empty`'s error.
///
/// The builder is explicit because the pin's default (`NoSuchElementError`) is
/// the caller's error type here; a source failure passes through untouched.
pub fn head<A, E>(items: Exit<Vec<A>, E>, on_empty: impl FnOnce() -> E) -> Exit<A, E> {
    match items {
        Exit::Success(values) => match values.into_iter().next() {
            Some(first) => Exit::Success(first),
            None => Exit::Failure(Cause::fail(on_empty())),
        },
        Exit::Failure(cause) => Exit::Failure(cause),
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

/// `Effect.all` at the pin, run from inside a fiber: the same fan-out and
/// fail-fast as [`all`], with the caller's fiber as the parent.
///
/// The two things [`all`] cannot do are what an interrupt row needs: the
/// workers are children (so an interrupt of the parent reaches them) and the
/// caller notices its own interrupt between deliveries, so a batch ends
/// `interrupted` instead of returning whatever it had collected.
///
/// Fail-fast rides on the result channel, not on join order: a failing body
/// that finishes early interrupts the siblings that are still running, even
/// while an earlier body has not answered yet.
pub fn all_in<A, E>(
    ctx: &Ctx,
    bodies: Vec<Body<A, E>>,
    concurrency: Concurrency,
) -> Exit<Vec<A>, E>
where
    A: Send + 'static,
    E: Send + 'static,
{
    match concurrency {
        Concurrency::Sequential | Concurrency::Bounded(0) => all_sequential_in(ctx, bodies),
        Concurrency::Bounded(n) => all_bounded_in(ctx, bodies, n),
        Concurrency::Unbounded => all_bounded_in(ctx, bodies, usize::MAX),
    }
}

fn all_sequential_in<A, E>(ctx: &Ctx, bodies: Vec<Body<A, E>>) -> Exit<Vec<A>, E>
where
    A: Send + 'static,
    E: Send + 'static,
{
    let mut out = Vec::with_capacity(bodies.len());
    for body in bodies {
        if ctx.is_interrupted() {
            return Exit::Failure(Cause::interrupt(None));
        }
        match ctx.fork_child(body, ForkOpts::immediate()).join() {
            Exit::Success(a) => out.push(a),
            Exit::Failure(cause) => return Exit::Failure(cause),
        }
    }
    Exit::Success(out)
}

fn all_bounded_in<A, E>(ctx: &Ctx, bodies: Vec<Body<A, E>>, bound: usize) -> Exit<Vec<A>, E>
where
    A: Send + 'static,
    E: Send + 'static,
{
    let total = bodies.len();
    if total == 0 {
        return Exit::Success(Vec::new());
    }
    let bound = bound.min(total).max(1);
    enum Done<A, E> {
        Ok(usize, A),
        Failed(usize, Cause<E>),
    }
    let (tx, rx) = std::sync::mpsc::channel::<Done<A, E>>();
    let mut queue: std::collections::VecDeque<(usize, Body<A, E>)> =
        bodies.into_iter().enumerate().collect();
    let mut live: std::collections::HashMap<usize, FiberHandle<(), std::convert::Infallible>> =
        std::collections::HashMap::new();
    let mut results: Vec<Option<A>> = (0..total).map(|_| None).collect();
    // Indexes whose body reported over the channel. A body that panicked never
    // reports: the supervisor turns the panic into a Die on the handle, so the
    // scan below is what notices it instead of waiting forever.
    let mut delivered: std::collections::HashSet<usize> = std::collections::HashSet::new();
    let mut finished = 0usize;
    // How many bodies were actually started. The loop below ends on this, not
    // on `total`: after a failure the queued bodies are dropped, so waiting
    // for `total` answers would wait forever for workers that never existed.
    let mut launched = 0usize;
    let mut failure: Option<Cause<E>> = None;

    let launch_one = |queue: &mut std::collections::VecDeque<(usize, Body<A, E>)>,
                          live: &mut std::collections::HashMap<
        usize,
        FiberHandle<(), std::convert::Infallible>,
    >| -> bool {
        let Some((index, body)) = queue.pop_front() else {
            return false;
        };
        let tx = tx.clone();
        let handle = ctx.fork_child(
            move |c: &Ctx| {
                let exit = body(c);
                let _ = tx.send(match exit {
                    Exit::Success(a) => Done::Ok(index, a),
                    Exit::Failure(cause) => Done::Failed(index, cause),
                });
                Exit::<(), std::convert::Infallible>::succeed(())
            },
            ForkOpts::immediate(),
        );
        live.insert(index, handle);
        true
    };

    // Stop the batch on `cause`: interrupt the workers still running, join
    // them so their cleanups finish, and fold both the cleanup defects and any
    // interrupt that landed meanwhile into the same cause.
    let stop_batch = |live: &mut std::collections::HashMap<
        usize,
        FiberHandle<(), std::convert::Infallible>,
    >,
                      cause: &mut Cause<E>,
                      finished: &mut usize,
                      launched: usize| {
        for handle in live.values() {
            handle.interrupt_unsafe();
        }
        let mut defects = Vec::new();
        for (_, handle) in live.drain() {
            if let Exit::Failure(sibling) = handle.join() {
                defects.extend(sibling.defects().into_iter().cloned());
            }
        }
        cause.append_defects(defects);
        if ctx.is_interrupted() && !cause.is_interrupted() {
            cause.reasons.push(Reason::Interrupt { fiber_id: None });
        }
        *finished = launched;
    };


    while live.len() < bound && launch_one(&mut queue, &mut live) {
        launched += 1;
    }

    while finished < launched {
        if ctx.is_interrupted() {
            // An interrupt from outside does not erase what the batch already
            // knows: a failure recorded before it keeps its reasons, and the
            // cleanup defects of the workers it stops are appended too (an
            // interrupt and a defect can share one cause).
            let mut cause = Cause::interrupt(None);
            for pending in rx.try_iter() {
                if let Done::Failed(_, sibling) = pending {
                    cause.reasons.extend(sibling.reasons);
                }
            }
            for handle in live.values() {
                handle.interrupt_unsafe();
            }
            let mut defects = Vec::new();
            for (_, handle) in live.drain() {
                if let Exit::Failure(sibling) = handle.join() {
                    defects.extend(sibling.defects().into_iter().cloned());
                }
            }
            cause.append_defects(defects);
            return Exit::Failure(cause);
        }
        match rx.recv_timeout(std::time::Duration::from_millis(5)) {
            Ok(Done::Ok(index, value)) => {
                delivered.insert(index);
                // Join rather than drop: the body reports over the channel
                // before its supervisor closes the worker's scope, so a
                // cleanup that dies is only visible on this exit. Dropping the
                // handle loses the defect and lets the batch claim a success
                // whose release died.
                let spilled = match live.remove(&index).map(|handle| handle.join()) {
                    Some(Exit::Failure(cause)) => {
                        cause.defects().into_iter().cloned().collect::<Vec<_>>()
                    }
                    _ => Vec::new(),
                };
                if !spilled.is_empty() && failure.is_none() {
                    let mut cause = Cause::from_defects(spilled);
                    stop_batch(&mut live, &mut cause, &mut finished, launched);
                    failure = Some(cause);
                    continue;
                }
                if failure.is_none() {
                    results[index] = Some(value);
                    if launch_one(&mut queue, &mut live) {
                        launched += 1;
                    }
                }
                finished += 1;
            }
            Ok(Done::Failed(index, cause)) => {
                delivered.insert(index);
                let mut cause = cause;
                // The failing body's own cleanup runs after it reported, so its
                // defects are read back off the exit here, not off the message.
                if let Some(Exit::Failure(tail)) = live.remove(&index).map(|handle| handle.join()) {
                    cause.append_defects(tail.defects().into_iter().cloned().collect());
                }
                if failure.is_none() {
                    // The failure wins, but the siblings it stops still get to
                    // run their cleanups, and a cleanup that dies rides along
                    // in the same cause instead of vanishing.
                    stop_batch(&mut live, &mut cause, &mut finished, launched);
                    failure = Some(cause);
                    continue;
                }
                finished += 1;
            }
            // Nothing delivered in this slice: loop so the interrupt check
            // above can run while the batch is quiet, and so a queued body
            // starts as soon as a slot frees.
            Err(_) => {
                // A finished handle that never reported is a body that
                // panicked (or was killed): its exit is the cause.
                let silent: Vec<usize> = live
                    .iter()
                    .filter(|(index, handle)| !delivered.contains(*index) && handle.is_done())
                    .map(|(index, _)| *index)
                    .collect();
                for index in silent {
                    let Some(handle) = live.remove(&index) else {
                        continue;
                    };
                    // The wrapper's own exit carries no typed failure (it
                    // always succeeds after sending), so only its defects can
                    // describe what went wrong.
                    let defects = match handle.join() {
                        Exit::Failure(cause) => {
                            cause.defects().into_iter().cloned().collect::<Vec<_>>()
                        }
                        // It did report after all; the channel will deliver it.
                        Exit::Success(_) => continue,
                    };
                    let mut cause: Cause<E> = Cause::from_defects(defects);
                    if cause.reasons.is_empty() && !ctx.is_interrupted() {
                        continue;
                    }
                    if failure.is_none() {
                        stop_batch(&mut live, &mut cause, &mut finished, launched);
                        failure = Some(cause);
                    }
                }
                if failure.is_some() {
                    continue;
                }
                while live.len() < bound && launch_one(&mut queue, &mut live) {
                    launched += 1;
                }
            }
        }
    }

    match failure {
        Some(cause) => Exit::Failure(cause),
        None => Exit::Success(
            results
                .into_iter()
                .map(|slot| slot.expect("every finished body recorded its value"))
                .collect(),
        ),
    }
}

/// `Effect.forEach` at the pin, run from inside a fiber: same fan-out as
/// [`for_each`], with the caller's fiber as the parent.
///
/// The difference is what an interrupt can do. Workers here are children of
/// `ctx`'s fiber, so an interrupt of the parent reaches them; the batch stops
/// launching once the parent is interrupted, and the first failure interrupts
/// the live siblings rather than waiting them out. Successes still come back
/// in input order however the workers interleave, because the join walks the
/// launch order.
///
/// [`for_each`] stays the detached spelling: it has no parent to be
/// interrupted through, which is why the interrupt rows use this one.
pub fn for_each_in<T, A, E>(
    ctx: &Ctx,
    items: Vec<T>,
    concurrency: Concurrency,
    f: impl Fn(T, &Ctx) -> Exit<A, E> + Send + Sync + 'static,
) -> Exit<Vec<A>, E>
where
    T: Send + 'static,
    A: Send + 'static,
    E: Send + 'static,
{
    let bound = match concurrency {
        Concurrency::Sequential | Concurrency::Bounded(0) => 1,
        Concurrency::Bounded(n) => n,
        Concurrency::Unbounded => usize::MAX,
    };
    let f: Arc<dyn Fn(T, &Ctx) -> Exit<A, E> + Send + Sync> = Arc::new(f);
    let mut pending = items.into_iter();
    let mut launched: std::collections::VecDeque<FiberHandle<A, E>> =
        std::collections::VecDeque::new();
    let mut out: Vec<A> = Vec::new();
    let mut interrupted = false;

    loop {
        while !interrupted && launched.len() < bound {
            let Some(item) = pending.next() else { break };
            if ctx.is_interrupted() {
                break;
            }
            let f = Arc::clone(&f);
            launched.push_back(
                ctx.fork_child(move |c: &Ctx| f(item, c), ForkOpts::immediate()),
            );
        }
        if ctx.is_interrupted() && !interrupted {
            interrupted = true;
            // Neither the queued items nor the live workers get to finish.
            pending.by_ref().for_each(drop);
            for handle in &launched {
                handle.interrupt_unsafe();
            }
        }
        let Some(handle) = launched.pop_front() else {
            break;
        };
        match handle.join() {
            Exit::Success(a) => out.push(a),
            Exit::Failure(cause) => {
                for handle in &launched {
                    handle.interrupt_unsafe();
                }
                return Exit::Failure(cause);
            }
        }
    }

    if interrupted {
        return Exit::Failure(Cause::interrupt(None));
    }
    Exit::Success(out)
}

/// `Effect.reduce` at the pin: fold items in order, sequentially, passing the
/// index. `init` is evaluated once per run, so a lazy zero stays lazy. Stops
/// at the first failure; the items after the failing one are not seen.
pub fn reduce<T, S, E, F0, F>(rt: &Runtime, items: Vec<T>, init: F0, f: F) -> Exit<S, E>
where
    T: Send + 'static,
    S: Send + 'static,
    E: Send + 'static,
    F0: Fn() -> S,
    F: Fn(S, T, usize) -> Exit<S, E> + Send + Sync + 'static,
{
    let f = Arc::new(f);
    let mut state = init();
    for (index, item) in items.into_iter().enumerate() {
        let (f, current) = (Arc::clone(&f), state);
        let body: Body<S, E> = Box::new(move |_ctx: &Ctx| f(current, item, index));
        match rt.fork_detach_with(body, ForkOpts::immediate()).join() {
            Exit::Success(next) => state = next,
            Exit::Failure(cause) => return Exit::Failure(cause),
        }
    }
    Exit::Success(state)
}

/// `Effect.findFirst` at the pin: return the first element whose predicate
/// passes, then stop. Elements after the match are never visited, and a
/// predicate failure fails the run.
pub fn find_first<T, E, F>(rt: &Runtime, items: Vec<T>, f: F) -> Exit<Option<T>, E>
where
    T: Clone + Send + 'static,
    E: Send + 'static,
    F: Fn(T, usize) -> Exit<bool, E> + Send + Sync + 'static,
{
    let f = Arc::new(f);
    for (index, item) in items.into_iter().enumerate() {
        let (f, probe) = (Arc::clone(&f), item.clone());
        let body: Body<bool, E> = Box::new(move |_ctx: &Ctx| f(probe, index));
        match rt.fork_detach_with(body, ForkOpts::immediate()).join() {
            Exit::Success(true) => return Exit::Success(Some(item)),
            Exit::Success(false) => continue,
            Exit::Failure(cause) => return Exit::Failure(cause),
        }
    }
    Exit::Success(None)
}

/// `Effect.findFirstFilter` at the pin: return the first successful mapped
/// value, then stop. A `None` mapping is a skip, not a failure.
pub fn find_first_filter<T, U, E, F>(rt: &Runtime, items: Vec<T>, f: F) -> Exit<Option<U>, E>
where
    T: Send + 'static,
    U: Send + 'static,
    E: Send + 'static,
    F: Fn(T, usize) -> Option<U> + Send + Sync + 'static,
{
    let f = Arc::new(f);
    for (index, item) in items.into_iter().enumerate() {
        let f = Arc::clone(&f);
        let body: Body<Option<U>, E> = Box::new(move |_ctx: &Ctx| Exit::Success(f(item, index)));
        match rt.fork_detach_with(body, ForkOpts::immediate()).join() {
            Exit::Success(Some(value)) => return Exit::Success(Some(value)),
            Exit::Success(None) => continue,
            Exit::Failure(cause) => return Exit::Failure(cause),
        }
    }
    Exit::Success(None)
}

/// `Effect.partition(items, f)` at the pin: map each item through `f` and
/// collect typed successes and failures in order. Defects and interrupts
/// still fail fast through [`partition`].
pub fn partition_items<T, A, E, F>(
    rt: &Runtime,
    items: Vec<T>,
    concurrency: Concurrency,
    f: F,
) -> Exit<(Vec<A>, Vec<E>), E>
where
    T: Send + 'static,
    A: Send + 'static,
    E: Send + 'static,
    F: Fn(T, &Ctx) -> Exit<A, E> + Send + Sync + 'static,
{
    partition(rt, item_bodies(items, f), concurrency)
}

/// `Effect.validate(items, f)` at the pin: every typed failure accumulates
/// into the error list, which is the failure channel itself.
pub fn validate_items<T, A, E, F>(
    rt: &Runtime,
    items: Vec<T>,
    concurrency: Concurrency,
    f: F,
) -> Exit<Vec<A>, Vec<E>>
where
    T: Send + 'static,
    A: Send + 'static,
    E: Send + 'static,
    F: Fn(T, &Ctx) -> Exit<A, E> + Send + Sync + 'static,
{
    validate(rt, item_bodies(items, f), concurrency)
}

/// `Effect.validate(items, f, { discard: true })`: `()` when everything
/// passes, every collected error otherwise.
pub fn validate_discard_items<T, A, E, F>(
    rt: &Runtime,
    items: Vec<T>,
    concurrency: Concurrency,
    f: F,
) -> Exit<(), Vec<E>>
where
    T: Send + 'static,
    A: Send + 'static,
    E: Send + 'static,
    F: Fn(T, &Ctx) -> Exit<A, E> + Send + Sync + 'static,
{
    validate_discard(rt, item_bodies(items, f), concurrency)
}

/// Build one fiber body per item (the pin's data-first collection form).
fn item_bodies<T, A, E, F>(items: Vec<T>, f: F) -> Vec<Body<A, E>>
where
    T: Send + 'static,
    A: Send + 'static,
    E: Send + 'static,
    F: Fn(T, &Ctx) -> Exit<A, E> + Send + Sync + 'static,
{
    let f = Arc::new(f);
    items
        .into_iter()
        .map(|item| {
            let f = Arc::clone(&f);
            Box::new(move |ctx: &Ctx| f(item, ctx)) as Body<A, E>
        })
        .collect()
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
    partition_with_deadline(rt, bodies, concurrency, BOUNDED_RECV_TIMEOUT)
}

/// Like [`partition`] with a caller-supplied bound on each fan-out wait.
/// A stalled wait fails with a defect naming the missing indexes; live
/// siblings are interrupted and detached, never joined.
pub fn partition_with_deadline<A, E>(
    rt: &Runtime,
    bodies: Vec<Body<A, E>>,
    concurrency: Concurrency,
    deadline: crate::Duration,
) -> Exit<(Vec<A>, Vec<E>), E>
where
    A: Send + 'static,
    E: Send + 'static,
{
    let std_deadline = std::time::Duration::from_millis(deadline.as_millis());
    match concurrency {
        Concurrency::Sequential => partition_sequential(rt, bodies),
        Concurrency::Bounded(n) => partition_bounded(rt, bodies, n, std_deadline),
        Concurrency::Unbounded => partition_bounded(rt, bodies, usize::MAX, std_deadline),
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
    deadline: std::time::Duration,
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
        match rx.recv_timeout(deadline) {
            Ok(Done::Ok(index, a)) => {
                live.remove(&index);
                if failure.is_none() {
                    passes[index] = Some(a);
                    spawn_one(&mut pending, &mut live);
                }
                finished += 1;
            }
            Ok(Done::Failed(index, cause)) => {
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
            Err(_) => {
                // Stalled wait: name the missing bodies, interrupt the live,
                // and detach. No join: an unkillable body would hang it too.
                let missing: Vec<usize> = passes
                    .iter()
                    .zip(fails.iter())
                    .enumerate()
                    .filter(|(_, (p, f))| p.is_none() && f.is_none())
                    .map(|(i, _)| i)
                    .collect();
                for handle in live.values() {
                    handle.interrupt_unsafe();
                }
                return Exit::Failure(Cause::die(Defect::new(format!(
                    "bounded fan-out timed out after {deadline:?} waiting for bodies {missing:?}"
                ))));
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
