//! Law L1: an [`Effect`] is a value describing work, not work itself.
//!
//! The [`Step`] state machine is private: user code can build descriptions
//! but never matches on them. Interpretation happens in exactly one place,
//! [`Effect::run_sync`], which runs descriptions with no async operations to
//! completion. The fiber/async executor (Phase 1, runtime bead) drives this
//! same machine.

use crate::{Cause, Defect, Exit, Provide, Reason};
use std::any::Any;
use std::collections::HashSet;
use std::marker::PhantomData;
use std::sync::Arc;

pub(crate) mod sealed {
    pub trait Sealed {}
    impl Sealed for () {}
}

/// Marker for the environment an effect needs. Only `()` exists in this wave;
/// the tuple machinery lands in Phase 2. Sealed so downstream crates cannot
/// invent requirements the interpreter does not understand.
pub trait Requirement: sealed::Sealed + Send + 'static {}
impl Requirement for () {}

/// A value travelling through the trampoline. The concrete type is known to
/// the [`Frame`] that resumes it, never to the loop, which is why the loop can
/// run a `flat_map` chain of any depth on one stack frame per step.
pub(crate) struct Erased(Box<dyn Any + Send>);

/// The next thing the interpreter should do.
pub(crate) enum Drive<E> {
    /// A value is ready for the innermost pending frame.
    Done(Erased),
    /// Nothing below this can run: unwind every pending frame.
    Fail(Cause<E>),
    /// Run `advance` next; its value resumes `frame`.
    Pending {
        advance: Advance<E>,
        frame: Box<dyn Frame<E>>,
    },
}

/// One pending continuation, held by the loop instead of the call stack.
pub(crate) trait Frame<E>: Send {
    fn resume(self: Box<Self>, value: Erased) -> Drive<E>;
}

/// A thunk the loop calls to advance one step. It owns the concrete types the
/// loop cannot name, and returns without recursing.
pub(crate) type Advance<E> = Box<dyn FnOnce() -> Drive<E> + Send>;

/// Runs one erased step to completion. Object-safe so `FlatMap`/`FoldE`
/// can hide their input type; implemented only inside this module, plus the
/// few combinators that own their own loop.
pub(crate) trait ErasedStep<A: Send + 'static, E: Send + 'static>: Send {
    fn run_erased(self: Box<Self>, scope: &crate::service::Services) -> Result<A, Cause<E>>;

    /// Take one step under the interpreter loop.
    ///
    /// The default runs `run_erased` and reports its answer, which is right
    /// for every atomic step. A step that would otherwise interpret a child by
    /// recursion - `flat_map` is the one that matters - answers
    /// [`Drive::Pending`] instead, so nesting depth costs stack depth nowhere.
    fn drive(self: Box<Self>, scope: &crate::service::Services) -> Drive<E> {
        match self.run_erased(scope) {
            Ok(a) => Drive::Done(Erased(Box::new(a))),
            Err(cause) => Drive::Fail(cause),
        }
    }
}

struct FlatMap<D: Send + 'static, A, E: Send + 'static, R: Requirement, F> {
    first: Effect<D, E, R>,
    cont: F,
    env: crate::service::Services,
    marker: PhantomData<fn() -> A>,
}

impl<D, A, E, R, F> ErasedStep<A, E> for FlatMap<D, A, E, R, F>
where
    D: Send + 'static,
    A: Send + 'static,
    E: Send + 'static,
    R: Requirement,
    F: FnOnce(D) -> Effect<A, E, R> + Send + 'static,
{
    fn run_erased(self: Box<Self>, scope: &crate::service::Services) -> Result<A, Cause<E>> {
        let Self {
            first, cont, env, ..
        } = *self;
        let inner = crate::service::merged_services(scope, &env);
        match first.run_sync_with(&inner) {
            Ok(d) => cont(d).run_sync_with(&inner),
            Err(cause) => Err(cause),
        }
    }

    fn drive(self: Box<Self>, scope: &crate::service::Services) -> Drive<E> {
        let Self {
            first, cont, env, ..
        } = *self;
        let inner = crate::service::merged_services(scope, &env);
        let advance_env = Arc::clone(&inner);
        Drive::Pending {
            advance: Box::new(move || enter_effect(first, &advance_env)),
            frame: Box::new(ContFrame {
                cont,
                env: inner,
                marker: PhantomData,
            }),
        }
    }
}

/// The `cont` half of a `flat_map`, waiting for `first`'s value.
struct ContFrame<D: Send + 'static, A, E: Send + 'static, R: Requirement, F> {
    cont: F,
    env: crate::service::Services,
    marker: PhantomData<fn() -> (A, E, R, D)>,
}

impl<D, A, E, R, F> Frame<E> for ContFrame<D, A, E, R, F>
where
    D: Send + 'static,
    A: Send + 'static,
    E: Send + 'static,
    R: Requirement,
    F: FnOnce(D) -> Effect<A, E, R> + Send + 'static,
{
    fn resume(self: Box<Self>, value: Erased) -> Drive<E> {
        let Self { cont, env, .. } = *self;
        // The loop only ever hands a frame the value its own step produced, so
        // a mismatch is an interpreter bug, reported as a defect rather than
        // a panic a caller could not act on.
        let Ok(value) = value.0.downcast::<D>() else {
            return Drive::Fail(Cause::die(Defect::new(
                "interpreter resumed a continuation with the wrong value type",
            )));
        };
        let next = cont(*value);
        enter_effect(next, &env)
    }
}

/// The pin's `Effect.suspend`: build the description on interpretation.
struct Defer<A, E, R: Requirement, F> {
    build: F,
    marker: PhantomData<fn() -> (A, E, R)>,
}

impl<A, E, R, F> ErasedStep<A, E> for Defer<A, E, R, F>
where
    A: Send + 'static,
    E: Send + 'static,
    R: Requirement,
    F: FnOnce() -> Effect<A, E, R> + Send + 'static,
{
    fn run_erased(self: Box<Self>, scope: &crate::service::Services) -> Result<A, Cause<E>> {
        (self.build)().run_sync_with(scope)
    }

    fn drive(self: Box<Self>, scope: &crate::service::Services) -> Drive<E> {
        enter_effect((self.build)(), scope)
    }
}

struct FoldE<A: Send + 'static, E: Send + 'static, B, E2, R: Requirement, FS, FF> {
    first: Effect<A, E, R>,
    on_success: FS,
    on_failure: FF,
    env: crate::service::Services,
    marker: PhantomData<fn() -> (B, E2)>,
}

impl<A, E, B, E2, R, FS, FF> ErasedStep<B, E2> for FoldE<A, E, B, E2, R, FS, FF>
where
    A: Send + 'static,
    E: Send + 'static,
    B: Send + 'static,
    E2: Send + 'static,
    R: Requirement,
    FS: FnOnce(A) -> Effect<B, E2, R> + Send + 'static,
    FF: FnOnce(Cause<E>) -> Effect<B, E2, R> + Send + 'static,
{
    fn run_erased(self: Box<Self>, scope: &crate::service::Services) -> Result<B, Cause<E2>> {
        let Self {
            first,
            on_success,
            on_failure,
            env,
            ..
        } = *self;
        let inner = crate::service::merged_services(scope, &env);
        match first.run_sync_with(&inner) {
            Ok(a) => on_success(a).run_sync_with(&inner),
            Err(cause) => on_failure(cause).run_sync_with(&inner),
        }
    }
}

/// `Effect.catchCause` at the pin, as its own step rather than a [`FoldE`].
///
/// The pin delivers an externally requested interrupt at the fiber's boundary,
/// so the handler is never offered a cause that only says "you were
/// interrupted" (`catch + ensuring + interrupt` is the row that pins it).
/// This port's fold runs the inner effect to completion and hands every cause
/// to the handler, so the rule has to live in the step: a cause carrying an
/// interrupt, seen while the fiber itself carries an interrupt request, is
/// re-raised unhandled. `ensuring` and `onExit` stay on [`FoldE`] and keep
/// observing the interrupt, which is the other half of that row.
struct CatchCause<A: Send + 'static, E: Send + 'static, R: Requirement, F> {
    inner: Effect<A, E, R>,
    on_cause: F,
    env: crate::service::Services,
}

impl<A, E, R, F> ErasedStep<A, E> for CatchCause<A, E, R, F>
where
    A: Send + 'static,
    E: Send + 'static,
    R: Requirement,
    F: FnOnce(Cause<E>) -> Effect<A, E, R> + Send + 'static,
{
    fn run_erased(self: Box<Self>, scope: &crate::service::Services) -> Result<A, Cause<E>> {
        let Self {
            inner,
            on_cause,
            env,
        } = *self;
        let services = crate::service::merged_services(scope, &env);
        match inner.run_sync_with(&services) {
            Ok(value) => Ok(value),
            Err(cause) => {
                if cause.is_interrupted()
                    && ambient_fiber(&services).is_some_and(|fiber| fiber.is_interrupted())
                {
                    return Err(cause);
                }
                on_cause(cause).run_sync_with(&services)
            }
        }
    }
}

/// Runs an inner effect under a scope minted for that run, then closes the
/// scope. Every exit path closes: success, typed failure, defect, interrupt.
#[cfg(feature = "tokio")]
struct Scoped<A: Send + 'static, E: Send + 'static, R: Requirement> {
    inner: Effect<A, E, R>,
    env: crate::service::Services,
}

#[cfg(feature = "tokio")]
impl<A: Send + 'static, E: Send + 'static, R: Requirement> ErasedStep<A, E> for Scoped<A, E, R> {
    fn run_erased(self: Box<Self>, scope: &crate::service::Services) -> Result<A, Cause<E>> {
        let Self { inner, env } = *self;
        let outer = crate::service::merged_services(scope, &env);
        // Minted per run, never shared: a description is a value, so running
        // it twice must not let the first run's scope observe the second's.
        let fresh = Arc::new(crate::Scope::new(crate::fiber::next_id().0));
        let mut maps = (*outer).clone();
        maps.replace_tag(
            <crate::scope::ScopeTag as crate::ServiceTag>::ID,
            Arc::new(Arc::clone(&fresh)),
        );
        let Effect { step, .. } = inner;
        let body = Effect {
            step,
            // The run carries no requirements of its own: the merged map
            // below is the whole environment, so nothing can be `Use`-read
            // that the parent did not already provide.
            services: crate::service::empty_services(),
            marker: PhantomData::<R>,
        };
        let out = body.run_sync_with(&Arc::new(maps));
        // Close after the run, on every path: the scope interrupts its
        // members and runs finalizers LIFO, exactly once.
        let spills = fresh.close();
        match out {
            Ok(value) => {
                if spills.is_empty() {
                    Ok(value)
                } else {
                    // A failed finalizer is a defect, never a typed failure:
                    // it did not come from the body's own error channel.
                    let mut spills = spills.into_iter();
                    let mut cause = Cause::die(spills.next().expect("checked non-empty"));
                    cause.reasons.extend(spills.map(Reason::Die));
                    Err(cause)
                }
            }
            Err(mut cause) => {
                // Append, never replace: the body's own reason survives (L6).
                cause.reasons.extend(spills.into_iter().map(Reason::Die));
                Err(cause)
            }
        }
    }
}

/// Hands the fiber running this effect to the callback, then runs the effect
/// the callback builds. The fiber comes from the ambient `FiberTag`; asking
/// where no fiber exists is a defect, the port's fail-closed answer to
/// missing context (`tx_retry` outside a transaction is the same shape).
#[cfg(feature = "tokio")]
struct WithFiber<A: Send + 'static, E: Send + 'static, R: Requirement, F> {
    f: F,
    marker: PhantomData<fn() -> (A, E, R)>,
}

#[cfg(feature = "tokio")]
impl<A, E, R, F> ErasedStep<A, E> for WithFiber<A, E, R, F>
where
    A: Send + 'static,
    E: Send + 'static,
    R: Requirement,
    F: FnOnce(&crate::fiber::Fiber) -> Effect<A, E, R> + Send + 'static,
{
    fn run_erased(self: Box<Self>, scope: &crate::service::Services) -> Result<A, Cause<E>> {
        let Self { f, .. } = *self;
        match scope.get_tag(<crate::fiber::FiberTag as crate::ServiceTag>::ID) {
            Some(any) => match any.downcast_ref::<crate::fiber::Fiber>() {
                Some(fiber) => f(fiber).run_sync_with(scope),
                None => Err(Cause::die(Defect::new("fiber tag holds a non-fiber value"))),
            },
            None => Err(Cause::die(Defect::new(
                "withFiber outside a fiber: run the effect through Ctx::spawn_effect or Runtime::spawn_effect",
            ))),
        }
    }
}

/// `Effect.forkChild` at the pin: fork the wrapped effect on a child of the
/// fiber running this one and hand its handle back without waiting. The
/// child inherits the parent's resolved services; its deferred start is the
/// pin's default (a `yieldNow`, an await, or the parent's own completion
/// fires it).
#[cfg(feature = "tokio")]
/// `Effect.acquireUseRelease` as a step.
#[cfg(feature = "tokio")]
struct AcquireUseRelease<A: Send + 'static, B: Send + 'static, E: Send + 'static, R: Requirement, U, Rel>
{
    acquire: Effect<A, E, R>,
    use_fn: U,
    release_fn: Rel,
    env: crate::service::Services,
    marker: PhantomData<fn() -> (A, B, E)>,
}

#[cfg(feature = "tokio")]
impl<A, B, E, R, U, Rel> ErasedStep<B, E> for AcquireUseRelease<A, B, E, R, U, Rel>
where
    A: Clone + Send + 'static,
    B: Send + 'static,
    E: PartialEq + Send + 'static,
    R: Requirement,
    U: FnOnce(A) -> Effect<B, E, R> + Send + 'static,
    Rel: FnOnce(A, &Exit<B, E>) -> Effect<(), E> + Send + 'static,
{
    fn run_erased(self: Box<Self>, scope: &crate::service::Services) -> Result<B, Cause<E>> {
        let Self {
            acquire,
            use_fn,
            release_fn,
            env,
            ..
        } = *self;
        let services = crate::service::merged_services(scope, &env);
        let fiber = ambient_fiber(&services);

        // Acquire: masked at the pin, so an interrupt requested here waits for
        // it to finish.
        let resource = match &fiber {
            Some(fiber) => fiber.with_masked(|| acquire.run_sync_with(&services)),
            None => acquire.run_sync_with(&services),
        }?;

        // Between the phases the use is interruptible. An interrupt that landed
        // during the masked acquire ends the region here: no use, still a
        // release. A caller mask keeps it pending instead, so the check only
        // fires when nothing is masking at this point.
        if let Some(live) = &fiber
            && live.is_interrupted()
            && !live.is_masked()
        {
            let use_exit: Exit<B, E> = Exit::Failure(Cause::interrupt(Some(live.id())));
            let released =
                live.with_masked(|| release_fn(resource, &use_exit).run_sync_with(&services));
            return match released {
                Ok(()) => Err(Cause::interrupt(Some(live.id()))),
                Err(release) => Err(release.combine(Cause::interrupt(Some(live.id())))),
            };
        }

        // Use: interruptible, and a panic is a defect the release can see.
        let used = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            use_fn(resource.clone()).run_sync_with(&services)
        }));
        let use_exit = match used {
            Ok(Ok(value)) => Exit::Success(value),
            Ok(Err(cause)) => Exit::Failure(cause),
            Err(payload) => Exit::Failure(Cause::die(crate::runtime::defect_from_panic(payload))),
        };
        // Release: masked at the pin, so it runs to completion after an
        // interrupt and is the last thing the region does.
        let released = match &fiber {
            Some(fiber) => fiber.with_masked(|| {
                release_fn(resource, &use_exit).run_sync_with(&services)
            }),
            None => release_fn(resource, &use_exit).run_sync_with(&services),
        };
        match (use_exit, released) {
            (Exit::Success(value), Ok(())) => Ok(value),
            (Exit::Success(_), Err(release)) => Err(release),
            (Exit::Failure(used), Ok(())) => Err(used),
            (Exit::Failure(used), Err(release)) => Err(used.combine(release)),
        }
    }
}

/// Where a forked fiber is bound beyond its parent: nowhere, the ambient
/// scope `Effect::scoped` installed, or a scope the caller named.
enum ForkScopes {
    None,
    Ambient,
    Given(std::sync::Arc<crate::Scope>),
}

struct ForkChild<A: Send + 'static, E: Send + 'static, R: Requirement> {
    inner: Effect<A, E, R>,
    /// `true` for the child forms: the child dies with this fiber. `false` for
    /// the daemon (`forkDetach`), which outlives it.
    parented: bool,
    scopes: ForkScopes,
    opts: crate::ForkOpts,
    marker: PhantomData<fn() -> (A, E, R)>,
}

#[cfg(feature = "tokio")]
impl<A, E, R> ErasedStep<crate::fiber::FiberHandle<A, E>, E> for ForkChild<A, E, R>
where
    A: Send + 'static,
    E: Send + 'static,
    R: Requirement,
{
    fn run_erased(
        self: Box<Self>,
        scope: &crate::service::Services,
    ) -> Result<crate::fiber::FiberHandle<A, E>, Cause<E>> {
        let Self {
            inner,
            parented,
            scopes,
            opts,
            ..
        } = *self;
        let Some(fiber) = ambient_fiber(scope) else {
            return Err(Cause::die(Defect::new(
                "forkChild outside a fiber: run the effect through Ctx::spawn_effect or Runtime::spawn_effect",
            )));
        };
        let scopes = match scopes {
            ForkScopes::None => Vec::new(),
            ForkScopes::Given(bound) => vec![bound],
            // `Effect::scoped` is the only thing that installs the tag, so a
            // missing one means the fork ran outside a scope: a defect, not a
            // silent root binding.
            ForkScopes::Ambient => match ambient_scope(scope) {
                Some(bound) => vec![bound],
                None => {
                    return Err(Cause::die(Defect::new(
                        "forkScoped outside a scope: wrap the effect in Effect::scoped",
                    )));
                }
            },
        };
        Ok(fiber.fork_effect_bound(inner, Arc::clone(scope), parented, scopes, opts))
    }
}

/// `Effect.awaitAllChildren` at the pin: wait for the children forked by the
/// wrapped effect, then hand its outcome back.
///
/// "Forked by the wrapped effect" is a snapshot taken before the body runs.
/// A child that already existed belongs to an enclosing scope, not to this
/// effect, so it is never awaited.
///
/// The wait polls the fiber's flag, so an interrupt ends it at once, unless a
/// masked region holds the depth (the pin's uninterruptible await waits the
/// children out, then exits interrupted). A body failure is kept and the
/// interrupt is appended to it, never substituted for it (L6).
///
/// Ceiling: `FiberControl` stores each child's exit erased, so a child's
/// failure is not re-raised by the parent; the pin does propagate it. No
/// ledger row exercises that path.
#[cfg(feature = "tokio")]
struct AwaitAllChildren<A: Send + 'static, E: Send + 'static, R: Requirement> {
    inner: Effect<A, E, R>,
    marker: PhantomData<fn() -> (A, E, R)>,
}

#[cfg(feature = "tokio")]
impl<A, E, R> ErasedStep<A, E> for AwaitAllChildren<A, E, R>
where
    A: Send + 'static,
    E: Send + 'static,
    R: Requirement,
{
    fn run_erased(self: Box<Self>, scope: &crate::service::Services) -> Result<A, Cause<E>> {
        let Self { inner, .. } = *self;
        let Some(fiber) = ambient_fiber(scope) else {
            return Err(Cause::die(Defect::new(
                "awaitAllChildren outside a fiber: run the effect through Ctx::spawn_effect or Runtime::spawn_effect",
            )));
        };
        let before: HashSet<u64> = fiber.children_snapshot().iter().map(|c| c.id.0).collect();
        let out = inner.run_sync_with(scope);
        loop {
            // Deferred children only run once their latch fires, and the await
            // is where the parent first yields to them.
            fiber.drain_pending();
            let waiting = fiber
                .children_snapshot()
                .iter()
                .any(|c| !before.contains(&c.id.0) && !c.is_done());
            // The wait ends when the children finish or when an interrupt
            // lands. Either way the outcome goes back unchanged: the
            // interpreter's own poll is what turns a pending interrupt into
            // the fiber's exit, so a body failure keeps its reasons and gains
            // the interrupt there (L6).
            if !waiting || (fiber.is_interrupted() && !fiber.is_masked()) {
                return out;
            }
            // ponytail: 1ms poll; a condvar notified by mark_done would
            // remove it, but that touches FiberControl's done pair.
            std::thread::park_timeout(std::time::Duration::from_millis(1));
        }
    }
}

/// `Effect.uninterruptible` at the pin: raise the fiber's interruptibility
/// depth around the wrapped effect. A request made inside still records; only
/// delivery waits for the depth to fall back to zero.
#[cfg(feature = "tokio")]
struct Uninterruptible<A: Send + 'static, E: Send + 'static, R: Requirement> {
    inner: Effect<A, E, R>,
    marker: PhantomData<fn() -> ()>,
}

#[cfg(feature = "tokio")]
impl<A, E, R> ErasedStep<A, E> for Uninterruptible<A, E, R>
where
    A: Send + 'static,
    E: Send + 'static,
    R: Requirement,
{
    fn run_erased(self: Box<Self>, scope: &crate::service::Services) -> Result<A, Cause<E>> {
        let Self { inner, .. } = *self;
        // The depth rides on the fiber itself and is lowered by a drop guard,
        // so a panic inside the region still restores it on unwind.
        match ambient_fiber(scope) {
            Some(fiber) => fiber.with_masked(|| inner.run_sync_with(scope)),
            // No fiber (a bare `run_sync`): nothing can interrupt the run, so
            // the region already is uninterruptible.
            None => inner.run_sync_with(scope),
        }
    }
}

/// `Effect.zip(..., { concurrent: true })` as a step.
#[cfg(feature = "tokio")]
struct ZipConcurrent<A, B, E, RA, RB>
where
    A: Send + 'static,
    B: Send + 'static,
    E: Send + 'static,
    RA: Requirement,
    RB: Requirement,
{
    left: Effect<A, E, RA>,
    right: Effect<B, E, RB>,
    env: crate::service::Services,
    marker: PhantomData<fn() -> (A, B)>,
}

#[cfg(feature = "tokio")]
impl<A, B, E, RA, RB> ErasedStep<(A, B), E> for ZipConcurrent<A, B, E, RA, RB>
where
    A: Send + 'static,
    B: Send + 'static,
    E: Send + 'static,
    RA: Requirement,
    RB: Requirement,
{
    fn run_erased(self: Box<Self>, scope: &crate::service::Services) -> Result<(A, B), Cause<E>> {
        let Self {
            left,
            right,
            env,
            ..
        } = *self;
        let services = crate::service::merged_services(scope, &env);
        let Some(fiber) = ambient_fiber(&services) else {
            return Err(Cause::die(Defect::new(
                "zip concurrent outside a fiber: run the effect through Ctx::spawn_effect or Runtime::spawn_effect",
            )));
        };
        enum Leg<A, B, E> {
            Left(Result<A, Cause<E>>),
            Right(Result<B, Cause<E>>),
        }
        let (tx, rx) = std::sync::mpsc::channel::<Leg<A, B, E>>();
        let tx_left = tx.clone();
        let left_handle = fiber.fork_child(
            move |c| {
                let ambient = c.services_with_fiber();
                let _ = tx_left.send(Leg::Left(left.run_sync_with(&ambient)));
                Exit::<(), std::convert::Infallible>::succeed(())
            },
            crate::ForkOpts::immediate(),
        );
        let right_handle = fiber.fork_child(
            move |c| {
                let ambient = c.services_with_fiber();
                let _ = tx.send(Leg::Right(right.run_sync_with(&ambient)));
                Exit::<(), std::convert::Infallible>::succeed(())
            },
            crate::ForkOpts::immediate(),
        );

        // First answer decides: a failure settles the pair and stops the
        // sibling; a success waits for the other leg's value.
        match rx.recv().expect("a zip leg always answers") {
            Leg::Left(Err(cause)) | Leg::Right(Err(cause)) => {
                left_handle.interrupt();
                right_handle.interrupt();
                let _ = left_handle.join();
                let _ = right_handle.join();
                Err(cause)
            }
            Leg::Left(Ok(a)) => match rx.recv().expect("the other zip leg answers") {
                Leg::Right(Ok(b)) => {
                    let _ = left_handle.join();
                    let _ = right_handle.join();
                    Ok((a, b))
                }
                Leg::Right(Err(cause)) => {
                    let _ = left_handle.join();
                    let _ = right_handle.join();
                    Err(cause)
                }
                Leg::Left(_) => unreachable!("the left leg answers once"),
            },
            Leg::Right(Ok(b)) => match rx.recv().expect("the other zip leg answers") {
                Leg::Left(Ok(a)) => {
                    let _ = left_handle.join();
                    let _ = right_handle.join();
                    Ok((a, b))
                }
                Leg::Left(Err(cause)) => {
                    let _ = left_handle.join();
                    let _ = right_handle.join();
                    Err(cause)
                }
                Leg::Right(_) => unreachable!("the right leg answers once"),
            },
        }
    }
}

/// `Effect.race` / `Effect.raceFirst` as a step.
///
/// Both legs run on children of the running fiber and the loser is interrupted
/// and joined before this returns, so a losing release finishes first — the
/// same bargain [`zip_concurrent`](Effect::zip_concurrent) strikes. The two
/// policies differ only in what counts as a settlement: `race` waits for a
/// success (failures are remembered, and two of them are concatenated),
/// `raceFirst` takes the first exit either way.
#[cfg(feature = "tokio")]
struct Race<A: Send + 'static, E: Send + 'static, R: Requirement> {
    left: Effect<A, E, R>,
    right: Effect<A, E, R>,
    first_settles: bool,
    env: crate::service::Services,
    marker: PhantomData<fn() -> (A, E)>,
}

/// Interrupt both legs and join whatever is left of them. A handle already
/// taken has had its exit read, so it is skipped.
#[cfg(feature = "tokio")]
fn stop_legs<A: Send + 'static, E: Send + 'static>(
    left: &mut Option<crate::fiber::FiberHandle<A, E>>,
    right: &mut Option<crate::fiber::FiberHandle<A, E>>,
) {
    for handle in [left.as_ref(), right.as_ref()].into_iter().flatten() {
        handle.interrupt_unsafe();
    }
    for handle in [left.take(), right.take()].into_iter().flatten() {
        let _ = handle.join();
    }
}

#[cfg(feature = "tokio")]
impl<A, E, R> ErasedStep<A, E> for Race<A, E, R>
where
    A: Send + 'static,
    E: Send + 'static,
    R: Requirement,
{
    fn run_erased(self: Box<Self>, scope: &crate::service::Services) -> Result<A, Cause<E>> {
        let Self {
            left,
            right,
            first_settles,
            env,
            ..
        } = *self;
        let services = crate::service::merged_services(scope, &env);
        let Some(fiber) = ambient_fiber(&services) else {
            return Err(Cause::die(Defect::new(
                "race outside a fiber: run the effect through Ctx::spawn_effect or Runtime::spawn_effect",
            )));
        };
        let mut left_handle = Some(fiber.fork_child(
            move |c| {
                let ambient = c.services_with_fiber();
                match left.run_sync_with(&ambient) {
                    Ok(value) => Exit::Success(value),
                    Err(cause) => Exit::Failure(cause),
                }
            },
            crate::ForkOpts::immediate(),
        ));
        let mut right_handle = Some(fiber.fork_child(
            move |c| {
                let ambient = c.services_with_fiber();
                match right.run_sync_with(&ambient) {
                    Ok(value) => Exit::Success(value),
                    Err(cause) => Exit::Failure(cause),
                }
            },
            crate::ForkOpts::immediate(),
        ));
        let mut left_cause: Option<Cause<E>> = None;
        let mut right_cause: Option<Cause<E>> = None;
        loop {
            for take_left in [true, false] {
                let handle = if take_left {
                    &left_handle
                } else {
                    &right_handle
                };
                if !handle.as_ref().is_some_and(|handle| handle.is_done()) {
                    continue;
                }
                let handle = if take_left {
                    left_handle.take()
                } else {
                    right_handle.take()
                };
                let Some(handle) = handle else { continue };
                match handle.join() {
                    Exit::Success(value) => {
                        stop_legs(&mut left_handle, &mut right_handle);
                        return Ok(value);
                    }
                    Exit::Failure(cause) => {
                        if first_settles {
                            stop_legs(&mut left_handle, &mut right_handle);
                            return Err(cause);
                        }
                        if take_left {
                            left_cause = Some(cause);
                        } else {
                            right_cause = Some(cause);
                        }
                    }
                }
            }
            if left_cause.is_some() && right_cause.is_some() {
                let mut cause = left_cause.take().expect("checked");
                cause
                    .reasons
                    .extend(right_cause.take().expect("checked").reasons);
                return Err(cause);
            }
            if fiber.is_interrupted() && !fiber.is_masked() {
                stop_legs(&mut left_handle, &mut right_handle);
                return Err(Cause::interrupt(Some(fiber.id())));
            }
            // ponytail: 1ms poll; a condvar signalled on child completion would
            // remove it, but that touches the driver.
            std::thread::park_timeout(std::time::Duration::from_millis(1));
        }
    }
}

/// The clock in scope, or the wall clock when nothing provided one.
#[cfg(feature = "tokio")]
fn ambient_clock(scope: &crate::service::Services) -> std::sync::Arc<dyn crate::ClockImpl> {
    use crate::ServiceTag;
    scope
        .get_tag(<crate::time::Clock as ServiceTag>::ID)
        .and_then(|any| {
            any.downcast_ref::<std::sync::Arc<dyn crate::ClockImpl>>()
                .cloned()
        })
        .unwrap_or_else(|| std::sync::Arc::new(crate::time::SystemClock) as std::sync::Arc<dyn crate::ClockImpl>)
}

/// Race an effect against the clock's deadline. `Ok(Some(value))` when the
/// effect finished first, `Ok(None)` when the deadline won, and the effect's
/// own cause when it failed. The loser is interrupted and joined before this
/// returns, so its finalizers have run by the time the caller sees the answer.
#[cfg(feature = "tokio")]
fn race_deadline<A, E, R>(
    inner: Effect<A, E, R>,
    duration: crate::Duration,
    services: &crate::service::Services,
) -> Result<Option<A>, Cause<E>>
where
    A: Send + 'static,
    E: Send + 'static,
    R: Requirement,
{
    // A value-level source is ready when the race starts, and the pin's race
    // gives a ready source the tie against a zero-duration deadline (row 1864).
    // Reading that off the step rather than polling for it keeps the answer
    // deterministic under load. `Sync` is deliberately not in this set: a thunk
    // may block, and a blocking body must still lose to its deadline.
    if matches!(inner.step, Step::Succeed(_) | Step::Fail(_)) {
        return inner.run_sync_with(services).map(Some);
    }
    let Some(fiber) = ambient_fiber(services) else {
        return Err(Cause::die(Defect::new(
            "timeout outside a fiber: run the effect through Ctx::spawn_effect or Runtime::spawn_effect",
        )));
    };
    let clock = ambient_clock(services);
    let deadline = clock
        .now_millis()
        .saturating_add(duration.as_millis());
    let child = fiber.fork_child(
        move |c| {
            let ambient = c.services_with_fiber();
            match inner.run_sync_with(&ambient) {
                Ok(value) => Exit::Success(value),
                Err(cause) => Exit::Failure(cause),
            }
        },
        crate::ForkOpts::immediate(),
    );
    loop {
        // The deadline is read before the child's completion, not after: once
        // the timeout has fired the source has lost, and a source that lands a
        // moment later must not win the answer (row 1878). A source that is
        // already done is handled by the ready-step check above.
        if clock.now_millis() >= deadline {
            child.interrupt();
            let _ = child.join();
            return Ok(None);
        }
        if child.is_done() {
            return match child.join() {
                Exit::Success(value) => Ok(Some(value)),
                Exit::Failure(cause) => Err(cause),
            };
        }
        if fiber.is_interrupted() && !fiber.is_masked() {
            child.interrupt();
            let _ = child.join();
            return Err(Cause::interrupt(Some(fiber.id())));
        }
        // ponytail: 1ms poll; the clock has no notification channel to wait on.
        std::thread::park_timeout(std::time::Duration::from_millis(1));
    }
}

/// `Effect.timeoutOption` as a step.
#[cfg(feature = "tokio")]
struct TimeoutOption<A: Send + 'static, E: Send + 'static, R: Requirement> {
    inner: Effect<A, E, R>,
    duration: crate::Duration,
    env: crate::service::Services,
    marker: PhantomData<fn() -> (A, E)>,
}

#[cfg(feature = "tokio")]
impl<A, E, R> ErasedStep<Option<A>, E> for TimeoutOption<A, E, R>
where
    A: Send + 'static,
    E: Send + 'static,
    R: Requirement,
{
    fn run_erased(self: Box<Self>, scope: &crate::service::Services) -> Result<Option<A>, Cause<E>> {
        let Self {
            inner,
            duration,
            env,
            ..
        } = *self;
        let services = crate::service::merged_services(scope, &env);
        race_deadline(inner, duration, &services)
    }
}

/// `Effect.timeoutOrElse` as a step.
#[cfg(feature = "tokio")]
struct TimeoutOrElse<A: Send + 'static, E: Send + 'static, R: Requirement> {
    inner: Effect<A, E, R>,
    duration: crate::Duration,
    fallback: Effect<A, E, R>,
    env: crate::service::Services,
    marker: PhantomData<fn() -> (A, E)>,
}

#[cfg(feature = "tokio")]
impl<A, E, R> ErasedStep<A, E> for TimeoutOrElse<A, E, R>
where
    A: Send + 'static,
    E: Send + 'static,
    R: Requirement,
{
    fn run_erased(self: Box<Self>, scope: &crate::service::Services) -> Result<A, Cause<E>> {
        let Self {
            inner,
            duration,
            fallback,
            env,
            ..
        } = *self;
        let services = crate::service::merged_services(scope, &env);
        match race_deadline(inner, duration, &services) {
            Ok(Some(value)) => Ok(value),
            Ok(None) => {
                let Some(fiber) = ambient_fiber(&services) else {
                    return Err(Cause::die(Defect::new(
                        "timeoutOrElse outside a fiber: run the effect through Ctx::spawn_effect or Runtime::spawn_effect",
                    )));
                };
                // The fallback runs on a child of the deadline's fiber, so an
                // interrupt that arrives while it is pending reaches it: the
                // row's "external interruption stops a pending fallback".
                let child = fiber.fork_child(
                    move |c| {
                        let ambient = c.services_with_fiber();
                        match fallback.run_sync_with(&ambient) {
                            Ok(value) => Exit::Success(value),
                            Err(cause) => Exit::Failure(cause),
                        }
                    },
                    crate::ForkOpts::immediate(),
                );
                loop {
                    if child.is_done() {
                        return match child.join() {
                            Exit::Success(value) => Ok(value),
                            Exit::Failure(cause) => Err(cause),
                        };
                    }
                    if fiber.is_interrupted() && !fiber.is_masked() {
                        child.interrupt();
                        let _ = child.join();
                        return Err(Cause::interrupt(Some(fiber.id())));
                    }
                    // ponytail: 1ms poll; a condvar notified on child completion
                    // would remove it, but that touches the driver.
                    std::thread::park_timeout(std::time::Duration::from_millis(1));
                }
            }
            Err(cause) => Err(cause),
        }
    }
}

/// `Effect.forever` as a step: rebuild and run until a run fails, with the
/// repeat boundary as the interrupt delivery point.
#[cfg(feature = "tokio")]
struct Forever<A: Send + 'static, E: Send + 'static, R: Requirement, F> {
    build: F,
    env: crate::service::Services,
    marker: PhantomData<fn() -> (A, E, R)>,
}

#[cfg(feature = "tokio")]
impl<A, E, R, F> ErasedStep<A, E> for Forever<A, E, R, F>
where
    A: Send + 'static,
    E: Send + 'static,
    R: Requirement,
    F: FnMut() -> Effect<A, E, R> + Send + 'static,
{
    fn run_erased(self: Box<Self>, scope: &crate::service::Services) -> Result<A, Cause<E>> {
        let Self {
            mut build, env, ..
        } = *self;
        let services = crate::service::merged_services(scope, &env);
        loop {
            if let Some(fiber) = ambient_fiber(&services)
                && fiber.is_interrupted()
                && !fiber.is_masked()
            {
                return Err(Cause::interrupt(Some(fiber.id())));
            }
            match build().run_sync_with(&services) {
                Ok(_) => continue,
                Err(cause) => return Err(cause),
            }
        }
    }
}

/// `Effect.sleep` as a step: park on the ambient clock until the deadline or
/// an interrupt, whichever lands first.
///
/// The interrupt check is mask-aware. A park that only read the flag would end
/// the moment an interrupt was requested, so regions masked with
/// `uninterruptible` would be interruptible at every sleep and only
/// "uninterruptible" between steps. Here the deadline wins inside a mask and
/// the pending interrupt is delivered once the region ends.
#[cfg(feature = "tokio")]
struct Sleep {
    duration: crate::Duration,
}

/// Generic in `E` so a sleep fits any effect's error channel: its only failure
/// is interruption, which carries no typed error of its own.
#[cfg(feature = "tokio")]
impl<E: Send + 'static> ErasedStep<(), E> for Sleep {
    fn run_erased(self: Box<Self>, scope: &crate::service::Services) -> Result<(), Cause<E>> {
        let Self { duration } = *self;
        let clock = ambient_clock(scope);
        let fiber = ambient_fiber(scope);
        let outcome = match &fiber {
            Some(fiber) => clock.sleep(duration, &|| fiber.is_interrupted() && !fiber.is_masked()),
            // No fiber: nothing can request an interrupt, so the park is the
            // whole answer.
            None => clock.sleep(duration, &|| false),
        };
        match outcome {
            crate::time::SleepOutcome::Elapsed => Ok(()),
            crate::time::SleepOutcome::Interrupted => {
                Err(Cause::interrupt(fiber.as_ref().map(|fiber| fiber.id())))
            }
        }
    }
}

/// The handle `Effect::uninterruptible_mask` gives its callback: `restore` at
/// the pin. Copy so a callback can use it in more than one arm.
#[cfg(feature = "tokio")]
#[derive(Clone, Copy, Debug)]
pub struct Restore;

#[cfg(feature = "tokio")]
impl Restore {
    /// Run `effect` interruptible inside the masked region.
    pub fn apply<A, E, R>(self, effect: Effect<A, E, R>) -> Effect<A, E, R>
    where
        A: Send + 'static,
        E: Send + 'static,
        R: Requirement,
    {
        effect.interruptible()
    }
}

/// `Effect.interruptible` at the pin: lower the fiber's interruptibility depth
/// by one (never below zero) around the wrapped effect. This is the `restore`
/// half of `uninterruptibleMask`: inside an uninterruptible region it puts the
/// wrapped effect back in reach of an interrupt, and the depth comes back on
/// the way out.
#[cfg(feature = "tokio")]
struct Interruptible<A: Send + 'static, E: Send + 'static, R: Requirement> {
    inner: Effect<A, E, R>,
    marker: PhantomData<fn() -> ()>,
}

#[cfg(feature = "tokio")]
impl<A, E, R> ErasedStep<A, E> for Interruptible<A, E, R>
where
    A: Send + 'static,
    E: Send + 'static,
    R: Requirement,
{
    fn run_erased(self: Box<Self>, scope: &crate::service::Services) -> Result<A, Cause<E>> {
        let Self { inner, .. } = *self;
        match ambient_fiber(scope) {
            Some(fiber) => fiber.with_unmasked(|| inner.run_sync_with(scope)),
            // No fiber (a bare `run_sync`): there is no depth to lower and no
            // request can arrive, so the region already is interruptible.
            None => inner.run_sync_with(scope),
        }
    }
}

/// Ambient-value reader: innermost override (if any) to a value. Total by
/// construction: the default fills a missing entry.
pub(crate) type RefFn<A> =
    Box<dyn Fn(Option<std::sync::Arc<dyn Any + Send + Sync>>) -> A + Send + Sync>;

/// The private description language. Users never name this type.
pub(crate) enum Step<A, E> {
    Succeed(A),
    Fail(Cause<E>),
    Sync(Box<dyn FnOnce() -> A + Send>),
    Suspend(Box<dyn ErasedStep<A, E> + Send>),
    /// Resolve a service tag from the provisioned environment at
    /// interpretation. Unreachable when types check: `provide` is the only
    /// way to satisfy the `R` channel, so a missing entry becomes `Die`.
    /// `read` is monomorphized at `use_service`: downcast the shared value
    /// plus clone. It takes the `Arc` by value so no reference coercion
    /// stands between the map and the downcast.
    Use {
        id: &'static str,
        read: fn(std::sync::Arc<dyn Any + Send + Sync>) -> Option<A>,
    },
    /// Resolve an ambient [`Reference`](crate::Reference): the innermost
    /// override, else the default. Total: a missing entry is the default,
    /// never a failure.
    ReadRef {
        id: &'static str,
        apply: RefFn<A>,
    },
}

/// An effect: a lazy description of work producing `A`, failing with typed
/// `E`, needing environment `R`. Building or dropping one runs nothing.
///
/// Dropping an unused description warns, because a description that is
/// never run is work the programmer meant to happen and did not.
#[must_use = "an unused Effect is a description nobody runs; run it or store it"]
pub struct Effect<A: Send + 'static, E: Send + 'static, R: Requirement = ()> {
    pub(crate) step: Step<A, E>,
    pub(crate) services: crate::service::Services,
    pub(crate) marker: PhantomData<R>,
}

/// `Effect.sleep` lives on a concrete `Self` so the `Clock` requirement shows
/// up in its return type: a `Self` generic over `R` could not answer with a
/// different `R`, and the call site would be one inference hole.
#[cfg(feature = "tokio")]
impl<E: Send + 'static> Effect<(), E, crate::service::Cons<crate::time::Clock, ()>> {
    /// `Effect.sleep` at the pin: park until the ambient clock's deadline
    /// passes, or until an interrupt arrives outside a masked region.
    ///
    /// The `Clock` requirement mirrors `timeoutOption`: a caller that provided
    /// a `TestClock` parks on that one, and with nothing provided the run falls
    /// back to the wall clock.
    pub fn sleep(duration: crate::Duration) -> Self {
        Effect {
            step: Step::Suspend(Box::new(Sleep { duration })),
            services: crate::service::empty_services(),
            marker: PhantomData,
        }
    }
}

/// The resume handle `Effect::callback` hands its registration closure.
///
/// Clonable, so the completion can live on another thread. Completing twice is
/// harmless: the first answer is the one the effect keeps, which is the pin's
/// `resume` contract. A completion that arrives after the fiber was interrupted
/// is ignored, because the effect already answered.
#[cfg(feature = "tokio")]
pub struct Resume<A: Send + 'static, E: Send + 'static> {
    slot: Arc<(std::sync::Mutex<Option<Exit<A, E>>>, std::sync::Condvar)>,
}

#[cfg(feature = "tokio")]
impl<A: Send + 'static, E: Send + 'static> Clone for Resume<A, E> {
    fn clone(&self) -> Self {
        Self {
            slot: Arc::clone(&self.slot),
        }
    }
}

#[cfg(feature = "tokio")]
impl<A: Send + 'static, E: Send + 'static> Resume<A, E> {
    /// Answer the callback with a whole exit: success or failure.
    pub fn complete(&self, exit: Exit<A, E>) {
        let (lock, cvar) = &*self.slot;
        let mut slot = lock.lock().unwrap();
        if slot.is_none() {
            *slot = Some(exit);
            cvar.notify_all();
        }
    }

    /// Answer with a value: the common case.
    pub fn succeed(&self, value: A) {
        self.complete(Exit::Success(value));
    }

    /// Answer with a failure.
    pub fn fail(&self, cause: Cause<E>) {
        self.complete(Exit::Failure(cause));
    }
}

/// Run an `onExit`-family callback, answering the defect when it panicked.
///
/// The callback is taken exactly once (the slot is `None` afterwards). A
/// panic here is the port's spelling of the pin's `throw` inside a finalizer:
/// a defect that rides beside the body's own reasons instead of unwinding
/// past the caller's `exit()`.
fn run_finalizer<A, E, F>(
    slot: &std::sync::Arc<std::sync::Mutex<Option<F>>>,
    exit: &Exit<A, E>,
) -> Option<Defect>
where
    A: Send + 'static,
    E: Send + 'static,
    F: FnOnce(&Exit<A, E>) + Send + 'static,
{
    let callback = slot.lock().ok()?.take()?;
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| callback(exit))) {
        Ok(()) => None,
        Err(payload) => Some(crate::runtime::defect_from_panic(payload)),
    }
}

/// `Effect.callback` as a step: register, then park until the registration
/// answers or the fiber is interrupted.
///
/// The interrupt check is what ends the park: an answer may never come, and the
/// fiber still has to stop. Checking it before the parked answer means an
/// interrupt already requested is not overtaken by an answer arriving in the
/// same turn of the loop. Either way the interpreter's own boundary turns a
/// success that lands after an interrupt into the interrupt exit, so the fleet
/// outcomes are the ones the row asserts.
#[cfg(feature = "tokio")]
struct Callback<A: Send + 'static, E: Send + 'static, F> {
    register: F,
    marker: PhantomData<fn() -> (A, E)>,
}

#[cfg(feature = "tokio")]
impl<A, E, F> ErasedStep<A, E> for Callback<A, E, F>
where
    A: Send + 'static,
    E: Send + 'static,
    F: FnOnce(Resume<A, E>) + Send + 'static,
{
    fn run_erased(self: Box<Self>, scope: &crate::service::Services) -> Result<A, Cause<E>> {
        let Self { register, .. } = *self;
        let slot = Arc::new((std::sync::Mutex::new(None), std::sync::Condvar::new()));
        let fiber = ambient_fiber(scope);
        register(Resume {
            slot: Arc::clone(&slot),
        });
        let (lock, cvar) = &*slot;
        let mut parked = lock.lock().unwrap();
        loop {
            if let Some(fiber) = &fiber
                && fiber.is_interrupted()
                && !fiber.is_masked()
            {
                return Err(Cause::interrupt(Some(fiber.id())));
            }
            if let Some(exit) = parked.take() {
                return match exit {
                    Exit::Success(value) => Ok(value),
                    Exit::Failure(cause) => Err(cause),
                };
            }
            // ponytail: 1ms cap on the wait; a wake-up on interrupt would need
            // request_interrupt to notify this condvar. 1ms keeps the latency
            // bounded without touching FiberControl.
            let (guard, _) = cvar
                .wait_timeout(parked, std::time::Duration::from_millis(1))
                .unwrap();
            parked = guard;
        }
    }
}

/// `Effect.callback` lives on a concrete `Self`: like `sleep`, it is a
/// constructor, not a transformation of an existing effect.
#[cfg(feature = "tokio")]
impl<A: Send + 'static, E: Send + 'static> Effect<A, E, ()> {
    /// `Effect.callback` at the pin: hand the registration a [`Resume`] and
    /// park until it answers or the fiber is interrupted.
    ///
    /// Ceiling: the pin's registration may return an effect that runs when the
    /// fiber is interrupted (its `onInterrupt` cleanup); here the registration
    /// returns nothing, so a cleanup is the caller's `.on_interrupt(...)`.
    pub fn callback<F>(register: F) -> Self
    where
        F: FnOnce(Resume<A, E>) + Send + 'static,
    {
        Effect {
            step: Step::Suspend(Box::new(Callback {
                register,
                marker: PhantomData,
            })),
            services: crate::service::empty_services(),
            marker: PhantomData,
        }
    }
}

impl<A: Send + 'static, E: Send + 'static, R: Requirement> Effect<A, E, R> {
    /// An effect that succeeds immediately with `value`. Runs nothing until
    /// interpreted.
    pub fn succeed(value: A) -> Self {
        Self {
            step: Step::Succeed(value),
            services: crate::service::empty_services(),
            marker: PhantomData,
        }
    }

    /// An effect that fails immediately with a typed error.
    pub fn fail(error: E) -> Self {
        Self {
            step: Step::Fail(Cause::fail(error)),
            services: crate::service::empty_services(),
            marker: PhantomData,
        }
    }

    /// An effect that dies immediately with a defect. Never a typed error.
    pub fn die(defect: Defect) -> Self {
        Self {
            step: Step::Fail(Cause::die(defect)),
            services: crate::service::empty_services(),
            marker: PhantomData,
        }
    }

    /// An effect interrupted before doing anything. Never success, never a
    /// typed error: interruption is not failure.
    pub fn interrupt() -> Self {
        Self {
            step: Step::Fail(Cause::interrupt(None)),
            services: crate::service::empty_services(),
            marker: PhantomData,
        }
    }

    /// Rebuild from a cause without interpreting it. Private: only the
    /// `catch` family passes causes through.
    fn fail_with(cause: Cause<E>) -> Self {
        Self {
            step: Step::Fail(cause),
            services: crate::service::empty_services(),
            marker: PhantomData,
        }
    }

    /// `Effect.fromOption` at the pin: `Some` succeeds, `None` fails with the
    /// supplied error. The pin defaults to `NoSuchElementError`; this port
    /// takes the error builder explicitly, so no invented type stands in.
    pub fn from_option<F>(option: std::option::Option<A>, on_none: F) -> Self
    where
        F: FnOnce() -> E,
    {
        match option {
            std::option::Option::Some(value) => Effect::succeed(value),
            std::option::Option::None => Effect::fail(on_none()),
        }
    }

    /// `Effect.fromResult` at the pin: `Ok` succeeds, `Err` fails with its
    /// error. The typed error channel only; defects never enter here.
    pub fn from_result(result: std::result::Result<A, E>) -> Self {
        match result {
            std::result::Result::Ok(value) => Effect::succeed(value),
            std::result::Result::Err(error) => Effect::fail(error),
        }
    }

    /// `Effect.transposeOption` at the pin: `Some(effect)` maps the success
    /// into `Some`, `None` succeeds with `None`. The inner effect is not run
    /// until interpreted, like every description.
    pub fn transpose_option(
        option: std::option::Option<Self>,
    ) -> Effect<std::option::Option<A>, E, R> {
        match option {
            std::option::Option::Some(effect) => effect.map(std::option::Option::Some),
            std::option::Option::None => Effect::succeed(std::option::Option::None),
        }
    }

    /// `Effect.filter` at the pin: keep the iterable's elements the
    /// predicate accepts, in iteration order. Lazy like every description:
    /// the predicate runs at interpretation, not at construction.
    pub fn filter<I, P>(elements: I, mut predicate: P) -> Effect<Vec<A>, std::convert::Infallible>
    where
        I: IntoIterator<Item = A> + Send + 'static,
        P: FnMut(&A) -> bool + Send + 'static,
    {
        Effect::sync(move || elements.into_iter().filter(|a| predicate(a)).collect())
    }

    /// `Effect.filterMap` at the pin: the pin's `Filter` shape, a `Result`
    /// selector. `Ok` keeps the mapped value, `Err` drops the element.
    pub fn filter_map<B, X, I, S>(
        elements: I,
        mut selector: S,
    ) -> Effect<Vec<B>, std::convert::Infallible>
    where
        B: Send + 'static,
        I: IntoIterator<Item = A> + Send + 'static,
        S: FnMut(A) -> std::result::Result<B, X> + Send + 'static,
    {
        Effect::sync(move || {
            elements
                .into_iter()
                .filter_map(|a| selector(a).ok())
                .collect()
        })
    }

    /// `Effect.firstSuccessOf` at the pin: try each effect in order, take
    /// the first success, and fall through to the next on a clean typed
    /// failure (defects and interrupts pass through, as in `catch`). All
    /// effects failing leaves the last failure; an empty collection is a
    /// defect, matching the pin's message.
    pub fn first_success_of(effects: Vec<Self>) -> Self {
        let mut rest = effects.into_iter().rev();
        let Some(mut succeed_with) = rest.next() else {
            return Effect::die(Defect::new("Received an empty collection of effects"));
        };
        for effect in rest {
            succeed_with = effect.catch(move |_| succeed_with);
        }
        succeed_with
    }

    /// `Effect.onError` at the pin: run `finalizer` only on a clean typed
    /// `Fail`, then re-raise that failure unchanged. Defects and interrupts
    /// pass through untouched (the same fire condition as [`catch`]).
    pub fn on_error<F>(self, finalizer: F) -> Self
    where
        F: FnOnce() + Send + 'static,
    {
        self.catch(move |e| {
            finalizer();
            Effect::fail(e)
        })
    }

    /// `Effect.ensuring` at the pin: run `finalizer` on every exit —
    /// success, typed failure, defect, and interrupt — then pass the exit
    /// through unchanged. Deliberately simple: the finalizer is a plain
    /// side effect, not an effect, so its own failure cannot append to the
    /// cause (L6's append rule needs a merging error channel this port does
    /// not express). The rows this covers finalize with `Effect.sync`.
    pub fn ensuring<F>(self, finalizer: F) -> Self
    where
        F: Fn() + Send + Sync + 'static,
    {
        let fin = Arc::new(finalizer);
        self.fold_e(
            {
                let fin = Arc::clone(&fin);
                move |value| {
                    fin();
                    Effect::succeed(value)
                }
            },
            {
                let fin = Arc::clone(&fin);
                move |cause| {
                    fin();
                    Effect::fail_with(cause)
                }
            },
        )
    }

    /// `Effect.scoped` at the pin: run `self` in a fresh [`Scope`], close that
    /// scope when the run ends (success, typed failure, defect, or interrupt),
    /// and strip the scope tag from the requirements. The body reaches the
    /// fresh scope with `use_service::<ScopeTag>()`.
    ///
    /// Rust substitution for the pin's `Exclude<R, Scope>`: the tag is the
    /// head of `R`, so the requirement tuple is ordered by provision, as
    /// everywhere else in this crate.
    ///
    /// Two differences from the pin worth naming. The pin hands the exit to
    /// the close so a finalizer can inspect it; [`Scope`] finalizers here are
    /// exit-blind by construction. And interruption is cooperative here: the
    /// pin's `scoped` closes on an interrupt delivered mid-body, while this
    /// one closes as soon as the body's steps return the interrupt as a
    /// cause. A fiber interrupted while its effect is in flight reaches
    /// [`Ctx::add_finalizer`](crate::Ctx::add_finalizer) instead.
    #[cfg(feature = "tokio")]
    pub fn scoped<Rest>(self) -> Effect<A, E, Rest>
    where
        R: Provide<crate::scope::ScopeTag, Rest>,
        Rest: Requirement,
    {
        let env = Arc::clone(&self.services);
        Effect::suspend_op(
            Scoped {
                inner: self,
                env: Arc::clone(&env),
            },
            env,
        )
    }

    /// `Effect.withFiber` at the pin: hand the fiber currently running this
    /// effect to `f`, and run the effect `f` builds in the same environment.
    /// The pin's callback receives the fiber object; here it receives a
    /// [`Fiber`](crate::Fiber) handle (identity and the interrupt flag),
    /// because this port keeps no mutable per-fiber context for the pin's
    /// `fiber.context`.
    ///
    /// Where the fiber comes from: the spawn seams that run an effect on a
    /// fiber's behalf, [`Ctx::spawn_effect`](crate::Ctx::spawn_effect) and
    /// [`Runtime::spawn_effect`](crate::Runtime::spawn_effect). Outside those
    /// (a bare `run_sync`, or `Runtime::run`, which executes on the calling
    /// thread) there is no fiber, and asking is a defect rather than a
    /// silent stand-in. Nested `with_fiber` sees the same fiber.
    #[cfg(feature = "tokio")]
    pub fn with_fiber<F>(f: F) -> Self
    where
        F: FnOnce(&crate::fiber::Fiber) -> Effect<A, E, R> + Send + 'static,
    {
        Effect {
            step: Step::Suspend(Box::new(WithFiber {
                f,
                marker: PhantomData,
            })),
            services: crate::service::empty_services(),
            marker: PhantomData,
        }
    }

    /// `Effect.forkChild` at the pin: run this effect on a child fiber of the
    /// one executing the current effect, and hand back its handle at once.
    /// The child is deferred, as the pin's default is: it starts when the
    /// parent next yields (a join, an interrupt, an `await_all_children`, or
    /// the parent's own completion). It dies with the parent.
    ///
    /// Outside a fiber there is nothing to fork from, so asking is a defect.
    #[cfg(feature = "tokio")]
    pub fn fork_child(self) -> Effect<crate::fiber::FiberHandle<A, E>, E, R> {
        self.fork_with(true, ForkScopes::None, crate::ForkOpts::deferred())
    }

    /// `Effect.forkChild({ startImmediately: true })`: the child is eligible
    /// to run before the parent's next yield, which is what lets an interrupt
    /// the parent receives find the child already running (and run its
    /// finalizers).
    #[cfg(feature = "tokio")]
    pub fn fork_child_immediate(self) -> Effect<crate::fiber::FiberHandle<A, E>, E, R> {
        self.fork_with(true, ForkScopes::None, crate::ForkOpts::immediate())
    }

    /// `Effect.forkDetach` at the pin: a daemon child. It starts at once, it
    /// does not die with this fiber, and this fiber's completion does not wait
    /// for it.
    #[cfg(feature = "tokio")]
    pub fn fork_detach(self) -> Effect<crate::fiber::FiberHandle<A, E>, E, R> {
        self.fork_with(false, ForkScopes::None, crate::ForkOpts::immediate())
    }

    /// `Effect.forkIn(scope)` at the pin: a child bound to `scope`, so closing
    /// the scope interrupts it and runs its finalizers.
    #[cfg(feature = "tokio")]
    pub fn fork_in(
        self,
        scope: std::sync::Arc<crate::Scope>,
    ) -> Effect<crate::fiber::FiberHandle<A, E>, E, R> {
        self.fork_with(true, ForkScopes::Given(scope), crate::ForkOpts::immediate())
    }

    /// `Effect.forkScoped` at the pin: a child bound to the ambient scope.
    ///
    /// The scope comes from `Effect::scoped`, which is the only thing that
    /// installs [`ScopeTag`](crate::scope::ScopeTag); the pin's `Scope.provide`
    /// is that wrap here, since this port has no ambient service layer to
    /// provide a scope into.
    #[cfg(feature = "tokio")]
    pub fn fork_scoped(
        self,
    ) -> Effect<
        crate::fiber::FiberHandle<A, E>,
        E,
        crate::service::Cons<crate::scope::ScopeTag, R>,
    > {
        Effect {
            step: Step::Suspend(Box::new(ForkChild {
                inner: self,
                parented: true,
                scopes: ForkScopes::Ambient,
                opts: crate::ForkOpts::immediate(),
                marker: PhantomData,
            })),
            services: crate::service::empty_services(),
            marker: PhantomData,
        }
    }

    /// The shared body of the four `fork*` constructors: only the parent
    /// link, the bound scopes and the start mode differ.
    #[cfg(feature = "tokio")]
    fn fork_with(
        self,
        parented: bool,
        scopes: ForkScopes,
        opts: crate::ForkOpts,
    ) -> Effect<crate::fiber::FiberHandle<A, E>, E, R> {
        Effect {
            step: Step::Suspend(Box::new(ForkChild {
                inner: self,
                parented,
                scopes,
                opts,
                marker: PhantomData,
            })),
            services: crate::service::empty_services(),
            marker: PhantomData,
        }
    }

    /// `Effect.awaitAllChildren` at the pin: wait for the children this effect
    /// forked, then hand its value (or its cause) back. A child forked before
    /// this effect ran belongs to an enclosing scope and is not awaited.
    ///
    /// The wait is interruptible: an interrupt ends it at once and the fiber
    /// exits interrupted, except inside an uninterruptible region, which waits
    /// the children out and then exits interrupted.
    #[cfg(feature = "tokio")]
    pub fn await_all_children(self) -> Self {
        Effect {
            step: Step::Suspend(Box::new(AwaitAllChildren {
                inner: self,
                marker: PhantomData,
            })),
            services: crate::service::empty_services(),
            marker: PhantomData,
        }
    }

    /// `Effect.uninterruptible` at the pin: run this effect with the fiber's
    /// interruptibility depth raised by one. A single combinator, no region
    /// callback: the pin's `uninterruptibleMask(restore => ...)` needs a
    /// `restore` this port has no `interruptible` counterpart for yet.
    #[cfg(feature = "tokio")]
    pub fn uninterruptible(self) -> Self {
        Effect {
            step: Step::Suspend(Box::new(Uninterruptible {
                inner: self,
                marker: PhantomData,
            })),
            services: crate::service::empty_services(),
            marker: PhantomData,
        }
    }

    /// `Effect.zip(..., { concurrent: true })` at the pin: run both legs at
    /// once and pair their values.
    ///
    /// The first failure interrupts the other leg and wins, which is the
    /// difference from the sequential `zip` shape: a leg that never ends
    /// cannot keep a failed sibling from settling the run.
    #[cfg(feature = "tokio")]
    pub fn zip_concurrent<B, R2>(self, other: Effect<B, E, R2>) -> Effect<(A, B), E, R>
    where
        B: Send + 'static,
        R2: Requirement,
    {
        let env = std::sync::Arc::clone(&self.services);
        Effect::suspend_op(
            ZipConcurrent {
                left: self,
                right: other,
                env: std::sync::Arc::clone(&env),
                marker: PhantomData,
            },
            env,
        )
    }

    /// `Effect.race` at the pin: run both effects at once and answer with the
    /// first SUCCESS. A failure does not beat a later success; when both fail,
    /// their reasons are concatenated in that order. The loser is interrupted
    /// and released before this returns.
    #[cfg(feature = "tokio")]
    pub fn race(self, other: Effect<A, E, R>) -> Self {
        let env = std::sync::Arc::clone(&self.services);
        Effect::suspend_op(
            Race {
                left: self,
                right: other,
                first_settles: false,
                env: std::sync::Arc::clone(&env),
                marker: PhantomData,
            },
            env,
        )
    }

    /// `Effect.raceFirst` at the pin: the first settlement wins, success or
    /// failure, and the other side is interrupted and released.
    #[cfg(feature = "tokio")]
    pub fn race_first(self, other: Effect<A, E, R>) -> Self {
        let env = std::sync::Arc::clone(&self.services);
        Effect::suspend_op(
            Race {
                left: self,
                right: other,
                first_settles: true,
                env: std::sync::Arc::clone(&env),
                marker: PhantomData,
            },
            env,
        )
    }

    /// `Effect.timeoutOption` at the pin: run the effect against the clock
    /// service's deadline. `Some(value)` when it finishes first, `None` when
    /// the deadline wins (the body is interrupted and released before this
    /// returns).
    #[cfg(feature = "tokio")]
    pub fn timeout_option(
        self,
        duration: crate::Duration,
    ) -> Effect<Option<A>, E, crate::service::Cons<crate::time::Clock, R>> {
        let env = std::sync::Arc::clone(&self.services);
        Effect::suspend_op(
            TimeoutOption {
                inner: self,
                duration,
                env: std::sync::Arc::clone(&env),
                marker: PhantomData,
            },
            env,
        )
    }

    /// `Effect.timeoutOrElse` at the pin: same race as
    /// [`timeout_option`](Self::timeout_option), but the deadline runs the
    /// caller's own effect — which is how a timeout dies or fails on its own
    /// terms instead of being mapped to `None`, and how a pending fallback can
    /// still be interrupted.
    ///
    /// The fallback shares this effect's `R`, so it reads the same services.
    #[cfg(feature = "tokio")]
    pub fn timeout_or_else(
        self,
        duration: crate::Duration,
        fallback: Effect<A, E, R>,
    ) -> Effect<A, E, crate::service::Cons<crate::time::Clock, R>> {
        let env = std::sync::Arc::clone(&self.services);
        Effect::suspend_op(
            TimeoutOrElse {
                inner: self,
                duration,
                fallback,
                env: std::sync::Arc::clone(&env),
                marker: PhantomData,
            },
            env,
        )
    }

    /// `Effect.forever` at the pin: run the effect the factory builds, over
    /// and over, until one run fails.
    ///
    /// A factory rather than a value because this port's descriptions are
    /// one-shot (a step owns its continuation); the pin can reuse a value.
    ///
    /// The repeat boundary is where an interrupt lands: a body may be
    /// uninterruptible, but it cannot make the next iteration start.
    pub fn forever<F>(build: F) -> Self
    where
        F: FnMut() -> Effect<A, E, R> + Send + 'static,
    {
        let env = crate::service::empty_services();
        Effect::suspend_op(
            Forever {
                build,
                env: std::sync::Arc::clone(&env),
                marker: PhantomData,
            },
            env,
        )
    }

    /// `Effect.uninterruptibleMask` at the pin: run `f` masked and hand it a
    /// [`Restore`] that puts one effect back in reach of an interrupt.
    ///
    /// The pin's callback shape is kept because it is the only way to express
    /// "masked except here": a plain `uninterruptible` around a plain
    /// `interruptible` cannot name the single region.
    #[cfg(feature = "tokio")]
    pub fn uninterruptible_mask<F>(f: F) -> Self
    where
        F: FnOnce(Restore) -> Self + Send + 'static,
    {
        Effect::suspend(move || f(Restore)).uninterruptible()
    }

    /// `Effect.interruptible` at the pin: the `restore` half of an
    /// uninterruptible region. The depth drops by one for the wrapped effect
    /// and comes back afterwards, so an interrupt requested while masked is
    /// delivered as soon as the region is entered.
    #[cfg(feature = "tokio")]
    pub fn interruptible(self) -> Self {
        Effect {
            step: Step::Suspend(Box::new(Interruptible {
                inner: self,
                marker: PhantomData,
            })),
            services: crate::service::empty_services(),
            marker: PhantomData,
        }
    }

    /// `Effect.acquireUseRelease` at the pin: acquire a resource, run `use`
    /// with it, then release it whatever the use did.
    ///
    /// Masking follows the pin phase by phase: the acquire and the release are
    /// uninterruptible, the use is not. So an interrupt that lands during the
    /// acquire still completes it, the use is then skipped, and the release
    /// runs anyway (row 315). A caller that wraps the whole thing in
    /// `uninterruptible` defers that interrupt past the use instead (row 370).
    ///
    /// The release receives the resource and the use's [`Exit`], which is how
    /// the pin lets a cleanup react to how the body ended. A panicking use
    /// becomes a defect in that exit, and the release still runs. When both
    /// the use and the release fail, both are kept (the pin's `Cause.combine`).
    ///
    /// `E: PartialEq` because merging two failures is `Cause::combine`, which
    /// dedups equal reasons - the pin's own `Equal`/`Hash` rule.
    pub fn acquire_use_release<B, U, Rel>(self, use_fn: U, release_fn: Rel) -> Effect<B, E, R>
    where
        A: Clone + Send + 'static,
        B: Send + 'static,
        E: PartialEq,
        U: FnOnce(A) -> Effect<B, E, R> + Send + 'static,
        Rel: FnOnce(A, &Exit<B, E>) -> Effect<(), E> + Send + 'static,
    {
        let env = std::sync::Arc::clone(&self.services);
        Effect::suspend_op(
            AcquireUseRelease {
                acquire: self,
                use_fn,
                release_fn,
                env: std::sync::Arc::clone(&env),
                marker: PhantomData,
            },
            env,
        )
    }

    /// `Effect.onExit` at the pin: observe the finished exit, then pass the
    /// original outcome through unchanged. The callback sees exactly the
    /// `Exit` a caller would (success, typed failure, defect, or interrupt),
    /// and the effect's own `A` and `E` are preserved. Exactly one call
    /// happens: the shared slot is taken in whichever arm runs.
    ///
    /// The callback borrows the exit rather than owning it, so no `Clone`
    /// bound on `E` is imposed (the pin hands the exit over by value; a
    /// shared reference carries the same observable claim).
    ///
    /// A panicking callback is a defect, the way a panicking body is: the
    /// panic is caught here and merged into the cause, so `onExit(throwing)`
    /// on a success is `Exit.die` and on an interrupt it is
    /// `[Interrupt, Die]` rather than an unwind past the caller's `exit()`.
    pub fn on_exit<F>(self, f: F) -> Self
    where
        F: FnOnce(&Exit<A, E>) + Send + 'static,
    {
        let slot = Arc::new(std::sync::Mutex::new(Some(f)));
        let (success_slot, failure_slot) = (Arc::clone(&slot), Arc::clone(&slot));
        self.fold_e(
            move |value| {
                let exit = Exit::Success(value);
                let defect = run_finalizer(&success_slot, &exit);
                match (exit, defect) {
                    (Exit::Success(a), None) => Effect::succeed(a),
                    (_, Some(defect)) => Effect::fail_with(Cause::die(defect)),
                    (Exit::Failure(_), None) => unreachable!("just built as success"),
                }
            },
            move |cause| {
                let exit = Exit::Failure(cause);
                let defect = run_finalizer(&failure_slot, &exit);
                match exit {
                    Exit::Failure(mut cause) => {
                        // The finalizer's own defect rides beside the body's
                        // reasons, after them, as the pin combines the two.
                        if let Some(defect) = defect {
                            cause.reasons.push(Reason::Die(defect));
                        }
                        Effect::fail_with(cause)
                    }
                    Exit::Success(_) => unreachable!("just built as failure"),
                }
            },
        )
    }

    /// `Effect.onErrorIf` at the pin: run the finalizer only on a failure
    /// whose cause passes `predicate`. Successes, and failures the
    /// predicate declines, skip it; the outcome passes through unchanged.
    pub fn on_error_if<P, F>(self, mut predicate: P, f: F) -> Self
    where
        P: FnMut(&Cause<E>) -> bool + Send + 'static,
        F: FnOnce(&Cause<E>) + Send + 'static,
    {
        self.on_exit(move |exit| {
            if let Exit::Failure(cause) = exit
                && predicate(cause)
            {
                f(cause);
            }
        })
    }

    /// `Effect.onErrorFilter` at the pin: the `Filter` shape over the
    /// failure cause. `Ok(pass)` hands the pass value and the cause to the
    /// finalizer; `Err` and success exits skip it.
    pub fn on_error_filter<B, X, S, F>(self, selector: S, f: F) -> Self
    where
        B: Send + 'static,
        S: FnOnce(&Cause<E>) -> std::result::Result<B, X> + Send + 'static,
        F: FnOnce(B, &Cause<E>) + Send + 'static,
    {
        self.on_exit(move |exit| {
            if let Exit::Failure(cause) = exit
                && let Ok(pass) = selector(cause)
            {
                f(pass, cause);
            }
        })
    }

    /// `Effect.onExitIf` at the pin: run the finalizer only when the exit
    /// passes `predicate`. The exit itself decides, so a success exit can
    /// carry its own finalizer.
    pub fn on_exit_if<P, F>(self, mut predicate: P, f: F) -> Self
    where
        P: FnMut(&Exit<A, E>) -> bool + Send + 'static,
        F: FnOnce(&Exit<A, E>) + Send + 'static,
    {
        self.on_exit(move |exit| {
            if predicate(exit) {
                f(exit);
            }
        })
    }

    /// `Effect.onExitFilter` at the pin: the `Filter` shape over the exit.
    /// `Ok(pass)` hands the pass value and the original exit to the
    /// finalizer; `Err` skips it.
    pub fn on_exit_filter<B, X, S, F>(self, selector: S, f: F) -> Self
    where
        B: Send + 'static,
        S: FnOnce(&Exit<A, E>) -> std::result::Result<B, X> + Send + 'static,
        F: FnOnce(B, &Exit<A, E>) + Send + 'static,
    {
        self.on_exit(move |exit| {
            if let Ok(pass) = selector(exit) {
                f(pass, exit);
            }
        })
    }

    /// Handle the typed error channel. Fires only on a clean single `Fail`;
    /// `Die`, `Interrupt`, and mixed causes pass through untouched.
    pub fn catch<F>(self, on_fail: F) -> Self
    where
        F: FnOnce(E) -> Self + Send + 'static,
    {
        self.fold_e(Effect::succeed, move |cause| {
            match cause.reasons.as_slice() {
                [Reason::Fail(_)] => {
                    let mut reasons = cause.reasons;
                    let e = match reasons.pop() {
                        Some(Reason::Fail(e)) => e,
                        _ => unreachable!(),
                    };
                    on_fail(e)
                }
                _ => Effect::fail_with(cause),
            }
        })
    }

    /// `Effect.catchIf` at the pin: recover a clean typed failure only when
    /// `predicate` accepts the error. A rejected error is re-raised, so the
    /// combinator never recovers an error it did not name.
    pub fn catch_if<P, F>(self, mut predicate: P, handler: F) -> Self
    where
        P: FnMut(&E) -> bool + Send + 'static,
        F: FnOnce(E) -> Self + Send + 'static,
    {
        self.catch(move |e| {
            if predicate(&e) {
                handler(e)
            } else {
                Effect::fail(e)
            }
        })
    }

    /// `Effect.catchIf` with the pin's third argument: a refinement that
    /// declines the error hands it to `or_else` instead of re-raising it.
    /// The declared error type already covers both branches, so `or_else`
    /// receives `E` and nothing is widened.
    pub fn catch_if_else<P, F, G>(self, mut predicate: P, handler: F, or_else: G) -> Self
    where
        P: FnMut(&E) -> bool + Send + 'static,
        F: FnOnce(E) -> Self + Send + 'static,
        G: FnOnce(E) -> Self + Send + 'static,
    {
        self.catch(move |e| {
            if predicate(&e) {
                handler(e)
            } else {
                or_else(e)
            }
        })
    }

    /// Handle defects. Fires only when the cause carries defects and no
    /// typed failure or interruption; those pass through untouched.
    pub fn catch_defect<F>(self, on_defects: F) -> Self
    where
        F: FnOnce(Vec<Defect>) -> Self + Send + 'static,
    {
        self.fold_e(Effect::succeed, move |cause| {
            let has_fail = cause.failure().is_some();
            if !has_fail && !cause.is_interrupted() && !cause.defects().is_empty() {
                let defects = cause.reasons.into_iter().filter_map(|r| match r {
                    Reason::Die(d) => Some(d),
                    _ => None,
                });
                on_defects(defects.collect())
            } else {
                Effect::fail_with(cause)
            }
        })
    }

    /// Handle the whole cause: the only combinator that sees `Fail`, `Die`,
    /// and `Interrupt` together. Library code does not call this.
    ///
    /// An interrupt requested from outside the effect ends it without being
    /// offered here; [`CatchCause`] carries the rule and the why.
    pub fn catch_cause<F>(self, on_cause: F) -> Self
    where
        F: FnOnce(Cause<E>) -> Self + Send + 'static,
    {
        let env = std::sync::Arc::clone(&self.services);
        Effect::suspend_op(
            CatchCause {
                inner: self,
                on_cause,
                env: std::sync::Arc::clone(&env),
            },
            env,
        )
    }

    /// `Effect.onInterrupt` at the pin: run `f` when the effect ends
    /// interrupted, then pass the exit through unchanged.
    ///
    /// Built on `on_exit` on purpose: an interrupt requested from outside ends
    /// the run without offering it to `catchCause`, but the exit still says
    /// it was interrupted, which is all this combinator needs.
    pub fn on_interrupt<F>(self, f: F) -> Self
    where
        F: FnOnce() + Send + 'static,
    {
        let slot = std::sync::Arc::new(std::sync::Mutex::new(Some(f)));
        self.on_exit(move |exit| {
            if let Exit::Failure(cause) = exit
                && cause.is_interrupted()
                && let Some(callback) = slot.lock().unwrap().take()
            {
                callback();
            }
        })
    }

    /// `Effect.sandbox` at the pin: reify the whole cause as a typed failure,
    /// so a caller can inspect a defect or an interrupt through the error
    /// channel.
    ///
    /// Deliberately on `fold_e`, not `catch_cause`: sandbox is the spelling
    /// that *must* see an interrupt, which is the opposite of the rule
    /// [`CatchCause`] enforces.
    pub fn sandbox(self) -> Effect<A, Cause<E>, R>
    where
        E: Send + 'static,
    {
        self.fold_e(Effect::succeed, Effect::fail)
    }

    /// `Effect.flip` at the pin: swap the success and the typed failure.
    ///
    /// Only a clean single typed failure becomes a value; a cause that also
    /// carries a defect or an interrupt has no `E` to hand back, so it stays
    /// a failure and names the reason it could not be flipped.
    pub fn flip(self) -> Effect<E, A, R>
    where
        A: Send + Clone + 'static,
        E: Send + Clone + 'static,
    {
        self.fold_e(
            |value: A| Effect::<E, A, R>::fail(value),
            |cause: Cause<E>| match cause.failure().cloned() {
                Some(error) => Effect::<E, A, R>::succeed(error),
                // Defects and interrupts have no `E` to hand back, so the flip
                // cannot be made: say so instead of inventing a value.
                None => Effect::<E, A, R>::die(Defect::new(
                    "flip applied to a cause with no single typed failure",
                )),
            },
        )
    }

    /// `Effect.catchCauseIf` at the pin: recover from any failure whose
    /// whole cause passes `predicate`. A declined cause is re-raised
    /// unchanged, every reason preserved.
    pub fn catch_cause_if<P, F>(self, mut predicate: P, handler: F) -> Self
    where
        P: FnMut(&Cause<E>) -> bool + Send + 'static,
        F: FnOnce(Cause<E>) -> Self + Send + 'static,
    {
        self.catch_cause(move |cause| {
            if predicate(&cause) {
                handler(cause)
            } else {
                Effect::fail_cause(cause)
            }
        })
    }

    /// `Effect.catchCauseFilter` at the pin: the pin's `Filter` shape, a
    /// `Result` selector over the cause. `Ok(pass)` routes the cause to the
    /// handler; `Err` declines and re-raises the cause unchanged.
    pub fn catch_cause_filter<B, X, S, H>(self, selector: S, handler: H) -> Self
    where
        B: Send + 'static,
        S: FnOnce(&Cause<E>) -> std::result::Result<B, X> + Send + 'static,
        H: FnOnce(B, Cause<E>) -> Self + Send + 'static,
    {
        self.catch_cause(move |cause| match selector(&cause) {
            Ok(pass) => handler(pass, cause),
            Err(_) => Effect::fail_cause(cause),
        })
    }

    /// `Effect.tapCauseIf` at the pin: observe the cause of any failure the
    /// predicate accepts, then re-raise it unchanged. A success never
    /// presents a cause, so the observer stays out of the way.
    pub fn tap_cause_if<P, F>(self, predicate: P, f: F) -> Self
    where
        P: FnMut(&Cause<E>) -> bool + Send + 'static,
        F: FnOnce(&Cause<E>) + Send + 'static,
    {
        self.catch_cause_if(predicate, move |cause| {
            f(&cause);
            Effect::fail_cause(cause)
        })
    }

    /// `Effect.tapCauseFilter` at the pin: the `Filter` shape over the
    /// cause. The observer receives the selector's pass value and the
    /// cause; a declined cause is re-raised without observation.
    pub fn tap_cause_filter<B, X, S, F>(self, selector: S, f: F) -> Self
    where
        B: Send + 'static,
        S: FnOnce(&Cause<E>) -> std::result::Result<B, X> + Send + 'static,
        F: FnOnce(B, &Cause<E>) + Send + 'static,
    {
        self.catch_cause(move |cause| match selector(&cause) {
            Ok(pass) => {
                f(pass, &cause);
                Effect::fail_cause(cause)
            }
            Err(_) => Effect::fail_cause(cause),
        })
    }

    /// An effect that runs `thunk` when interpreted, not when built. The thunk
    /// must be pure work: panics escape to the fiber supervisor, which turns
    /// them into [`Reason::Die`] at the boundary.
    pub fn sync<F>(thunk: F) -> Self
    where
        F: FnOnce() -> A + Send + 'static,
    {
        Self {
            step: Step::Sync(Box::new(thunk)),
            services: crate::service::empty_services(),
            marker: PhantomData,
        }
    }

    pub(crate) fn suspend_op(
        op: impl ErasedStep<A, E> + 'static,
        env: crate::service::Services,
    ) -> Self {
        Self {
            step: Step::Suspend(Box::new(op)),
            services: env,
            marker: PhantomData,
        }
    }

    /// `Effect.suspend` at the pin: build the description only when it is
    /// interpreted. That is what makes a recursive description finite to
    /// build, and what lets the interpreter run its chain without growing the
    /// call stack (row 2713).
    pub fn suspend<F>(build: F) -> Self
    where
        F: FnOnce() -> Effect<A, E, R> + Send + 'static,
    {
        Self::suspend_op(
            Defer {
                build,
                marker: PhantomData,
            },
            crate::service::empty_services(),
        )
    }

    /// Lazily apply `cont` to the success value. `cont` never runs on failure.
    pub fn flat_map<B, F>(self, cont: F) -> Effect<B, E, R>
    where
        B: Send + 'static,
        F: FnOnce(A) -> Effect<B, E, R> + Send + 'static,
    {
        let env = std::sync::Arc::clone(&self.services);
        Effect::suspend_op(
            FlatMap {
                first: self,
                cont,
                env: std::sync::Arc::clone(&env),
                marker: PhantomData,
            },
            env,
        )
    }

    /// Lazily apply `f` to the success value.
    pub fn map<B, F>(self, f: F) -> Effect<B, E, R>
    where
        B: Send + 'static,
        F: FnOnce(A) -> B + Send + 'static,
    {
        self.flat_map(move |a| Effect::succeed(f(a)))
    }

    /// Fold success and failure into a new effect, possibly with a different
    /// error channel. Private: only `result`, `exit`, and `or_die` narrow it.
    fn fold_e<B, E2, FS, FF>(self, on_success: FS, on_failure: FF) -> Effect<B, E2, R>
    where
        B: Send + 'static,
        E2: Send + 'static,
        FS: FnOnce(A) -> Effect<B, E2, R> + Send + 'static,
        FF: FnOnce(Cause<E>) -> Effect<B, E2, R> + Send + 'static,
    {
        let env = std::sync::Arc::clone(&self.services);
        Effect::suspend_op(
            FoldE {
                first: self,
                on_success,
                on_failure,
                env: std::sync::Arc::clone(&env),
                marker: PhantomData,
            },
            env,
        )
    }

    /// Fold the typed failure into the success channel as [`Err`]. Only a
    /// clean single `Fail` converts: defects and interrupts are preserved as
    /// failures, never smuggled into `Ok`. Needs [`std::fmt::Debug`] on `E`
    /// to render a failure that shares its cause with other reasons.
    pub fn result(self) -> Effect<Result<A, E>, std::convert::Infallible, R>
    where
        E: std::fmt::Debug,
    {
        self.fold_e(
            move |a| Effect::succeed(Ok(a)),
            move |cause| {
                if matches!(cause.reasons.as_slice(), [Reason::Fail(_)]) {
                    let mut reasons = cause.reasons;
                    let e = match reasons.pop() {
                        Some(Reason::Fail(e)) => e,
                        _ => unreachable!(),
                    };
                    Effect::succeed(Err(e))
                } else {
                    Effect::die_like(cause, "result")
                }
            },
        )
    }

    /// Reify the outcome as [`Exit`] data. Lossless: the cause keeps its `E`.
    pub fn exit(self) -> Effect<Exit<A, E>, std::convert::Infallible, R> {
        self.fold_e(
            move |a| Effect::succeed(Exit::Success(a)),
            move |cause| Effect::succeed(Exit::Failure(cause)),
        )
    }

    /// Erase the typed channel: failures become defects. `Die` and
    /// `Interrupt` pass through untouched; an interrupt is still not a
    /// failure after `or_die`.
    pub fn or_die(self) -> Effect<A, std::convert::Infallible, R>
    where
        E: std::fmt::Debug,
    {
        self.fold_e(
            move |a| Effect::succeed(a),
            move |cause| Effect::die_like(cause, "or_die"),
        )
    }

    /// `Effect.mapError` at the pin: translate the typed failure channel.
    ///
    /// The direction a service read needs. `use_service::<T>()` is
    /// `Infallible`, because resolving a tag either succeeds or is a defect;
    /// but the `Err` a `Program` declares is a real typed channel, and the
    /// two do not unify. Without this, `use_service` could not be composed
    /// into an [`Interpret`] at all, which is why
    /// [`Effect::provide_from`] needs it rather than standing alone.
    ///
    /// A defect or an interrupt is not a typed failure, so it passes through
    /// untouched: [`Cause::map`] keeps every non-`Fail` reason and every
    /// annotation verbatim, so the four outcomes stay apart.
    pub fn map_err<F, E2>(self, on_error: F) -> Effect<A, E2, R>
    where
        F: Fn(E) -> E2 + Send + 'static,
        E2: Send + 'static,
    {
        self.fold_e(
            move |a| Effect::succeed(a),
            move |cause| Effect::fail_cause(cause.map(on_error)),
        )
    }

    /// Re-raise a cause verbatim (`Effect.failCause` at the pin): every
    /// reason survives unchanged, so a `Fail` stays a typed failure and a
    /// `Die` stays a defect.
    ///
    /// Public because cause-observing combinators outside this crate (the
    /// stream `tapCause`/`catchDefect` family) must re-raise what they saw
    /// without collapsing the typed/defect distinction, which `die_like`
    /// deliberately does.
    pub fn fail_cause(cause: Cause<E>) -> Self {
        Self {
            step: Step::Fail(cause),
            services: crate::service::empty_services(),
            marker: PhantomData,
        }
    }

    /// Build a defect-channel effect from any cause: `Die`/`Interrupt`
    /// reasons survive as-is, `Fail` renders to a defect. Private: only
    /// `result` and `or_die` narrow the error channel.
    fn die_like(cause: Cause<E>, context: &'static str) -> Effect<A, std::convert::Infallible, R>
    where
        E: std::fmt::Debug,
    {
        let mut out = Cause::empty();
        for reason in cause.reasons {
            out.reasons.push(match reason {
                Reason::Fail(e) => Reason::Die(Defect::new(format!("{context}: {e:?}"))),
                Reason::Die(d) => Reason::Die(d),
                Reason::Interrupt { fiber_id } => Reason::Interrupt { fiber_id },
            });
        }
        Effect {
            step: Step::Fail(out),
            services: crate::service::empty_services(),
            marker: PhantomData,
        }
    }

    /// Interpret a description with no async operations to completion. The
    /// one place interpretation happens; `pub(crate)` so only this crate's
    /// executor (and its unit tests) can run descriptions.
    pub(crate) fn run_sync(self) -> Result<A, Cause<E>> {
        self.run_sync_with(&crate::service::empty_services())
    }

    pub(crate) fn run_sync_with(self, outer: &crate::service::Services) -> Result<A, Cause<E>> {
        let Effect { step, services, .. } = self;
        // The fiber whose interrupt flag this run observes. Read once: a tag
        // cannot change under a running effect, and a run outside a fiber has
        // none, so `Runtime::run` on the current thread is never interrupted.
        let fiber = ambient_fiber(outer);
        // Frames live here, not on the call stack: a description nested ten
        // thousand `flat_map`s deep runs in one loop.
        let mut frames: Vec<Box<dyn Frame<E>>> = Vec::new();
        // Poll before the first step, not only between steps. `enter_step` runs
        // a `Suspend` op (and its own nested run) on the spot, so without this
        // a pending interrupt would let one more effect start, which is the
        // "restore delivers the pending interrupt, the restored effect never
        // runs" row (Effect.test.ts:2337).
        if let Some(fiber) = &fiber
            && fiber.is_interrupted()
            && !fiber.is_masked()
        {
            return Err(Cause::interrupt(Some(fiber.id())));
        }
        let mut drive = enter_step(step, &services, outer);
        loop {
            // Poll between steps, never inside one: an atomic effect runs to
            // its own end, and an endless chain of `flat_map`s stops here. A
            // masked run (a release) finishes first (L7).
            //
            // A failure that is already in hand keeps its own reasons and
            // gains the interrupt; only a value is discarded. Replacing the
            // cause would lose the body's failure at exactly the boundary the
            // pin keeps both (a failure held while awaiting children).
            if let Some(fiber) = &fiber
                && fiber.is_interrupted()
                && !fiber.is_masked()
            {
                return match drive {
                    Drive::Fail(mut cause) => {
                        if !cause.is_interrupted() {
                            cause.reasons.push(Reason::Interrupt {
                                fiber_id: Some(fiber.id()),
                            });
                        }
                        Err(cause)
                    }
                    _ => Err(Cause::interrupt(Some(fiber.id()))),
                };
            }
            match drive {
                Drive::Fail(cause) => return Err(cause),
                Drive::Done(value) => match frames.pop() {
                    None => {
                        return match value.0.downcast::<A>() {
                            Ok(value) => Ok(*value),
                            // Unreachable when the types line up; a defect beats
                            // a panic a caller could not act on.
                            Err(_) => Err(Cause::die(Defect::new(
                                "interpreter finished with the wrong value type",
                            ))),
                        };
                    }
                    Some(frame) => drive = frame.resume(value),
                },
                Drive::Pending { advance, frame } => {
                    frames.push(frame);
                    drive = advance();
                }
            }
        }
    }
}

/// The fiber this run belongs to, when its ambient services carry one.
///
/// Only the spawn seams inject [`FiberTag`](crate::fiber::FiberTag), and
/// `provide` cannot forge it, so a tag found here is the fiber actually
/// executing the effect.
fn ambient_fiber(scope: &crate::service::Services) -> Option<crate::fiber::Fiber> {
    let any = scope.get_tag(<crate::fiber::FiberTag as crate::ServiceTag>::ID)?;
    any.downcast_ref::<crate::fiber::Fiber>().cloned()
}

/// The scope `Effect::scoped` installed for this run, if any.
#[cfg(feature = "tokio")]
fn ambient_scope(scope: &crate::service::Services) -> Option<std::sync::Arc<crate::Scope>> {
    let any = scope.get_tag(<crate::scope::ScopeTag as crate::ServiceTag>::ID)?;
    any.downcast_ref::<std::sync::Arc<crate::Scope>>().cloned()
}

/// One step of the interpreter: resolve the step's own services, then hand it
/// either a finished value or a continuation to run later.
fn enter_effect<A, E, R>(effect: Effect<A, E, R>, outer: &crate::service::Services) -> Drive<E>
where
    A: Send + 'static,
    E: Send + 'static,
    R: Requirement,
{
    let Effect { step, services, .. } = effect;
    enter_step(step, &services, outer)
}

fn enter_step<A, E>(
    step: Step<A, E>,
    services: &crate::service::Services,
    outer: &crate::service::Services,
) -> Drive<E>
where
    A: Send + 'static,
    E: Send + 'static,
{
    let scope = crate::service::merged_services(outer, services);
    // A tag provided twice with distinct values poisons the whole build:
    // any run Dies, even with no `Use` node left to observe it.
    if let Some(id) = scope.conflict() {
        return Drive::Fail(Cause::die(Defect::new(format!(
            "ambiguous tag: {id} (provided twice with distinct values)"
        ))));
    }
    match step {
        Step::Succeed(a) => Drive::Done(Erased(Box::new(a))),
        Step::Fail(cause) => Drive::Fail(cause),
        Step::Sync(thunk) => Drive::Done(Erased(Box::new(thunk()))),
        Step::Suspend(op) => op.drive(&scope),
        Step::Use { id, read } => match scope.get_tag(id) {
            Some(any) => match read(any) {
                Some(a) => Drive::Done(Erased(Box::new(a))),
                None => Drive::Fail(Cause::die(Defect::new(format!(
                    "service type mismatch for tag: {id}"
                )))),
            },
            None => Drive::Fail(Cause::die(Defect::new(format!("missing service: {id}")))),
        },
        Step::ReadRef { id, apply } => Drive::Done(Erased(Box::new(apply(scope.get_ref(id))))),
    }
}

/// An infallible description satisfies any error channel vacuously: the
/// step cannot fail, so the target `E` is a widening, not a claim. The
/// pin gets this from variance over `never`; Rust states it.
pub fn widen_infallible<E, A>(
    effect: Effect<A, std::convert::Infallible, ()>,
) -> Effect<A, E, ()>
where
    A: Send + 'static,
    E: Send + 'static,
{
    effect.map_err(|never| match never {})
}

#[cfg(feature = "stm")]
impl<A: Send + 'static, E: Send + 'static> Effect<A, E, ()> {
    /// `Effect.tx` at the pin: the outermost call opens a transaction
    /// boundary. Writes to transactional refs made inside the body stage
    /// on those refs and publish together when the body succeeds; a
    /// failure that escapes the boundary discards them. A nested `tx`
    /// inside a running boundary composes into it — the same journal, no
    /// inner boundary — so a failure caught inside the body is an
    /// ordinary value and its writes survive with it.
    ///
    /// A body that calls [`tx_retry`](Self::tx_retry), or whose read set
    /// went stale while it ran, is parked on its read set and run again
    /// when a commit moves it: the boundary's loop is optimistic
    /// concurrency, one attempt per commit of a ref it read.
    ///
    /// The body is a factory because this port's descriptions are
    /// single-shot: running consumes one, so an attempt loop needs a
    /// fresh build per attempt (the pin re-runs one value; Rust rebuilds
    /// it). The boundary is likewise staged per interpretation — running
    /// the same `tx` description twice opens two boundaries with two
    /// journals.
    pub fn tx<F>(build: F) -> Self
    where
        F: Fn() -> Effect<A, E, ()> + Send + 'static,
    {
        Effect::suspend_op(
            crate::stm::TxBoundary {
                build: Box::new(build),
                env: crate::service::empty_services(),
                marker: std::marker::PhantomData,
            },
            crate::service::empty_services(),
        )
    }

    /// `Effect.txRetry` at the pin: mark the running boundary for
    /// another attempt and interrupt the body. The boundary's loop parks
    /// on its read set and reruns when a commit moves it. Outside a
    /// boundary it is a defect, the port's fail-closed answer to missing
    /// context.
    pub fn tx_retry() -> Self {
        widen_infallible(crate::service::read_reference(&crate::stm::TX_CURRENT)).flat_map(
            |current| match current {
                Some(tx) => {
                    tx.set_retry();
                    Effect::interrupt()
                }
                None => Effect::die(Defect::new("txRetry outside a transaction")),
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::convert::Infallible;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn calls() -> (Arc<AtomicUsize>, impl Fn() + Send + 'static) {
        let n = Arc::new(AtomicUsize::new(0));
        let bump = {
            let n = n.clone();
            move || {
                n.fetch_add(1, Ordering::SeqCst);
            }
        };
        (n, bump)
    }

    #[test]
    fn sync_runs_thunk_to_value() {
        let e: Effect<i32, String> = Effect::sync(|| 6 * 7);
        assert_eq!(e.run_sync(), Ok(42));
    }

    #[test]
    fn descriptions_rerun_each_interpretation() {
        let (n, bump) = calls();
        let e: Effect<(), String> = Effect::sync(bump);
        assert_eq!(n.load(Ordering::SeqCst), 0);
        assert!(e.run_sync().is_ok());
        // Rebuild: effects are values, running consumes; a fresh build runs again.
        let (n2, bump2) = calls();
        let _ = n2;
        let e2: Effect<(), String> = Effect::sync(bump2);
        assert!(e2.run_sync().is_ok());
        assert_eq!(n.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn flat_map_composes_and_short_circuits() {
        let (n, bump) = calls();
        let e: Effect<i32, String> = Effect::succeed(21).flat_map(move |a| {
            bump();
            Effect::succeed(a * 2)
        });
        assert_eq!(e.run_sync(), Ok(42));
        assert_eq!(n.load(Ordering::SeqCst), 1);

        // Continuation never runs on failure; the cause survives.
        let (m, bump2) = calls();
        let f: Effect<i32, String> = Effect::fail("no".to_string()).flat_map(move |a: i32| {
            bump2();
            Effect::succeed(a)
        });
        assert_eq!(f.run_sync(), Err(Cause::fail("no".to_string())));
        assert_eq!(m.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn map_skips_defects() {
        let (n, bump) = calls();
        let e: Effect<i32, String> =
            Effect::<i32, String>::die(Defect::new("boom")).map(move |a| {
                bump();
                a + 1
            });
        let cause = e.run_sync().expect_err("die must stay failure");
        assert_eq!(cause.defects().len(), 1);
        assert_eq!(n.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn result_converts_only_clean_failures() {
        let ok: Effect<Result<i32, String>, Infallible> = Effect::succeed(1_i32).result();
        assert_eq!(ok.run_sync(), Ok(Ok(1)));

        let err: Effect<Result<i32, String>, Infallible> = Effect::fail("bad".to_string()).result();
        assert_eq!(err.run_sync(), Ok(Err("bad".to_string())));

        // A defect is never smuggled into Ok(Err(..)).
        let die: Effect<i32, String> = Effect::die(Defect::new("boom"));
        let r = die.result().run_sync().expect_err("die stays failure");
        assert_eq!(r.defects().len(), 1);
        assert!(r.failure().is_none());

        // Interrupts stay interrupts.
        let intr: Effect<i32, String> = Effect {
            step: Step::Fail(Cause::interrupt(None)),
            services: crate::service::empty_services(),
            marker: PhantomData,
        };
        let r = intr
            .result()
            .run_sync()
            .expect_err("interrupt stays failure");
        assert!(matches!(r.reasons.as_slice(), [Reason::Interrupt { .. }]));
    }

    #[test]
    fn exit_reifies_losslessly() {
        let s: Effect<Exit<i32, String>, Infallible> = Effect::succeed(7).exit();
        assert_eq!(s.run_sync(), Ok(Exit::Success(7)));

        let f: Effect<Exit<i32, String>, Infallible> = Effect::fail("x".to_string()).exit();
        assert_eq!(
            f.run_sync(),
            Ok(Exit::Failure(Cause::fail("x".to_string())))
        );
    }

    #[test]
    fn or_die_kills_types_not_signals() {
        let f: Effect<i32, String> = Effect::fail("bad".to_string());
        let r = f.or_die().run_sync().expect_err("fail becomes die");
        assert_eq!(r.defects().len(), 1);
        assert!(r.defects()[0].message.contains("bad"));

        let d: Effect<i32, String> = Effect::die(Defect::new("orig"));
        let r = d.or_die().run_sync().expect_err("die stays die");
        assert_eq!(r.defects()[0].message, "orig");

        let i: Effect<i32, String> = Effect {
            step: Step::Fail(Cause::interrupt(None)),
            services: crate::service::empty_services(),
            marker: PhantomData,
        };
        let r = i
            .or_die()
            .run_sync()
            .expect_err("interrupt stays interrupt");
        assert!(matches!(r.reasons.as_slice(), [Reason::Interrupt { .. }]));
    }
}
