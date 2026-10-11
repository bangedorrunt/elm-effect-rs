//! Law L1: an [`Effect`] is a value describing work, not work itself.
//!
//! The [`Step`] state machine is private: user code can build descriptions
//! but never matches on them. Interpretation happens in exactly one place,
//! [`Effect::run_sync`], which runs descriptions with no async operations to
//! completion. The fiber/async executor (Phase 1, runtime bead) drives this
//! same machine.

use crate::{Cause, Defect, Exit, Reason};
use std::any::Any;
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

/// Runs one erased step to completion. Object-safe so `FlatMap`/`FoldE`
/// can hide their input type; implemented only inside this module.
pub(crate) trait ErasedStep<A, E>: Send {
    fn run_erased(self: Box<Self>, scope: &crate::service::Services) -> Result<A, Cause<E>>;
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
    pub fn transpose_option(option: std::option::Option<Self>) -> Effect<
        std::option::Option<A>,
        E,
        R,
    > {
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

    /// `Effect.onExit` at the pin: observe the finished exit, then pass the
    /// original outcome through unchanged. The callback sees exactly the
    /// `Exit` a caller would (success, typed failure, defect, or interrupt),
    /// and the effect's own `A` and `E` are preserved. Exactly one call
    /// happens: the shared slot is taken in whichever arm runs.
    ///
    /// The callback borrows the exit rather than owning it, so no `Clone`
    /// bound on `E` is imposed (the pin hands the exit over by value; a
    /// shared reference carries the same observable claim).
    pub fn on_exit<F>(self, f: F) -> Self
    where
        F: FnOnce(&Exit<A, E>) + Send + 'static,
    {
        let slot = Arc::new(std::sync::Mutex::new(Some(f)));
        let (success_slot, failure_slot) = (Arc::clone(&slot), Arc::clone(&slot));
        self.fold_e(
            move |value| {
                let exit = Exit::Success(value);
                if let Some(cb) = success_slot.lock().unwrap().take() {
                    cb(&exit);
                }
                match exit {
                    Exit::Success(a) => Effect::succeed(a),
                    Exit::Failure(_cause) => unreachable!("just built as success"),
                }
            },
            move |cause| {
                let exit = Exit::Failure(cause);
                if let Some(cb) = failure_slot.lock().unwrap().take() {
                    cb(&exit);
                }
                match exit {
                    Exit::Failure(cause) => Effect::fail_with(cause),
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
            if let Exit::Failure(cause) = exit {
                if predicate(cause) {
                    f(cause);
                }
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
            if let Exit::Failure(cause) = exit {
                if let Ok(pass) = selector(cause) {
                    f(pass, cause);
                }
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
    pub fn catch_cause<F>(self, on_cause: F) -> Self
    where
        F: FnOnce(Cause<E>) -> Self + Send + 'static,
    {
        self.fold_e(Effect::succeed, on_cause)
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

    fn suspend(op: impl ErasedStep<A, E> + 'static, env: crate::service::Services) -> Self {
        Self {
            step: Step::Suspend(Box::new(op)),
            services: env,
            marker: PhantomData,
        }
    }

    /// Lazily apply `cont` to the success value. `cont` never runs on failure.
    pub fn flat_map<B, F>(self, cont: F) -> Effect<B, E, R>
    where
        B: Send + 'static,
        F: FnOnce(A) -> Effect<B, E, R> + Send + 'static,
    {
        let env = std::sync::Arc::clone(&self.services);
        Effect::suspend(
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
        Effect::suspend(
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
        let scope = crate::service::merged_services(outer, &self.services);
        // A tag provided twice with distinct values poisons the whole build:
        // any run Dies, even with no `Use` node left to observe it.
        if let Some(id) = scope.conflict() {
            return Err(Cause::die(Defect::new(format!(
                "ambiguous tag: {id} (provided twice with distinct values)"
            ))));
        }
        match self.step {
            Step::Succeed(a) => Ok(a),
            Step::Fail(cause) => Err(cause),
            Step::Sync(thunk) => Ok(thunk()),
            Step::Suspend(op) => op.run_erased(&scope),
            Step::Use { id, read } => match scope.get_tag(id) {
                Some(any) => match read(any) {
                    Some(a) => Ok(a),
                    None => Err(Cause::die(Defect::new(format!(
                        "service type mismatch for tag: {id}"
                    )))),
                },
                None => Err(Cause::die(Defect::new(format!("missing service: {id}")))),
            },
            Step::ReadRef { id, apply } => Ok(apply(scope.get_ref(id))),
        }
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
