//! Law L1: an [`Effect`] is a value describing work, not work itself.
//!
//! The [`Step`] state machine is private: user code can build descriptions
//! but never matches on them. Interpretation happens in exactly one place,
//! [`Effect::run_sync`], which runs descriptions with no async operations to
//! completion. The fiber/async executor (Phase 1, runtime bead) drives this
//! same machine.

use crate::{Cause, Defect, Exit, Reason};
use std::marker::PhantomData;

mod sealed {
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
trait ErasedStep<A, E, R>: Send {
    fn run_erased(self: Box<Self>) -> Result<A, Cause<E>>;
}

struct FlatMap<D: Send + 'static, A, E: Send + 'static, R: Requirement, F> {
    first: Effect<D, E, R>,
    cont: F,
    marker: PhantomData<fn() -> A>,
}

impl<D, A, E, R, F> ErasedStep<A, E, R> for FlatMap<D, A, E, R, F>
where
    D: Send + 'static,
    A: Send + 'static,
    E: Send + 'static,
    R: Requirement,
    F: FnOnce(D) -> Effect<A, E, R> + Send + 'static,
{
    fn run_erased(self: Box<Self>) -> Result<A, Cause<E>> {
        let Self { first, cont, .. } = *self;
        match first.run_sync() {
            Ok(d) => cont(d).run_sync(),
            Err(cause) => Err(cause),
        }
    }
}

struct FoldE<A: Send + 'static, E: Send + 'static, B, E2, R: Requirement, FS, FF> {
    first: Effect<A, E, R>,
    on_success: FS,
    on_failure: FF,
    marker: PhantomData<fn() -> (B, E2)>,
}

impl<A, E, B, E2, R, FS, FF> ErasedStep<B, E2, R> for FoldE<A, E, B, E2, R, FS, FF>
where
    A: Send + 'static,
    E: Send + 'static,
    B: Send + 'static,
    E2: Send + 'static,
    R: Requirement,
    FS: FnOnce(A) -> Effect<B, E2, R> + Send + 'static,
    FF: FnOnce(Cause<E>) -> Effect<B, E2, R> + Send + 'static,
{
    fn run_erased(self: Box<Self>) -> Result<B, Cause<E2>> {
        let Self {
            first,
            on_success,
            on_failure,
            ..
        } = *self;
        match first.run_sync() {
            Ok(a) => on_success(a).run_sync(),
            Err(cause) => on_failure(cause).run_sync(),
        }
    }
}

/// The private description language. Users never name this type.
enum Step<A, E, R> {
    Succeed(A),
    Fail(Cause<E>),
    Sync(Box<dyn FnOnce() -> A + Send>),
    Suspend(Box<dyn ErasedStep<A, E, R> + Send>),
}

/// An effect: a lazy description of work producing `A`, failing with typed
/// `E`, needing environment `R`. Building or dropping one runs nothing.
pub struct Effect<A: Send + 'static, E: Send + 'static, R: Requirement = ()> {
    step: Step<A, E, R>,
    marker: PhantomData<R>,
}

impl<A: Send + 'static, E: Send + 'static, R: Requirement> Effect<A, E, R> {
    /// An effect that succeeds immediately with `value`. Runs nothing until
    /// interpreted.
    pub fn succeed(value: A) -> Self {
        Self {
            step: Step::Succeed(value),
            marker: PhantomData,
        }
    }

    /// An effect that fails immediately with a typed error.
    pub fn fail(error: E) -> Self {
        Self {
            step: Step::Fail(Cause::fail(error)),
            marker: PhantomData,
        }
    }

    /// An effect that dies immediately with a defect. Never a typed error.
    pub fn die(defect: Defect) -> Self {
        Self {
            step: Step::Fail(Cause::die(defect)),
            marker: PhantomData,
        }
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
            marker: PhantomData,
        }
    }

    fn suspend(op: impl ErasedStep<A, E, R> + 'static) -> Self {
        Self {
            step: Step::Suspend(Box::new(op)),
            marker: PhantomData,
        }
    }

    /// Lazily apply `cont` to the success value. `cont` never runs on failure.
    pub fn flat_map<B, F>(self, cont: F) -> Effect<B, E, R>
    where
        B: Send + 'static,
        F: FnOnce(A) -> Effect<B, E, R> + Send + 'static,
    {
        Effect::suspend(FlatMap {
            first: self,
            cont,
            marker: PhantomData,
        })
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
        Effect::suspend(FoldE {
            first: self,
            on_success,
            on_failure,
            marker: PhantomData,
        })
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
            marker: PhantomData,
        }
    }

    /// Interpret a description with no async operations to completion. The
    /// one place interpretation happens; `pub(crate)` so only this crate's
    /// executor (and its unit tests) can run descriptions.
    pub(crate) fn run_sync(self) -> Result<A, Cause<E>> {
        match self.step {
            Step::Succeed(a) => Ok(a),
            Step::Fail(cause) => Err(cause),
            Step::Sync(thunk) => Ok(thunk()),
            Step::Suspend(op) => op.run_erased(),
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
            marker: PhantomData,
        };
        let r = i
            .or_die()
            .run_sync()
            .expect_err("interrupt stays interrupt");
        assert!(matches!(r.reasons.as_slice(), [Reason::Interrupt { .. }]));
    }
}
