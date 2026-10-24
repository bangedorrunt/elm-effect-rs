//! A [`Scope`] bounds fiber lifetimes and owns finalizers (Laws L6, L7).
//!
//! Two finalizer kinds share one LIFO stack, run exactly once:
//! - sync finalizers (`add_finalizer`), plain closures;
//! - effect finalizers (`add_finalizer_effect`): [`Effect`]s the runtime runs
//!   at close. A failing effect finalizer is returned as a [`Defect`]; the
//!   supervisor appends it to the fiber's cause without erasing the original
//!   reason (L6). Finalizers run masked: cancellation requested mid-release
//!   runs the release to completion, then takes effect (L7).
//!
//! Either kind can be exit-aware. A release registered with
//! [`Scope::add_finalizer_with_exit`] or
//! [`Scope::add_finalizer_effect_with_exit`] sees the scope's [`ScopeExit`]
//! and can commit or roll back on success, failure, or interruption. The exit
//! is erased of the success value and the typed error, because a release
//! needs the shape of the outcome (and its defects), not the `E` value.
//!
//! `Drop` closes the scope, so it is the release path only for resources
//! whose finalizers need no `.await`: everything here runs synchronously.

use std::backtrace::Backtrace;
use std::sync::{
    Arc, Mutex, Weak,
    atomic::{AtomicBool, Ordering},
};

use crate::fiber::FiberControl;
use crate::{Cause, Defect, Effect, Exit, Reason};

/// How long a close waits for interrupted members to finish their releases
/// before running the scope's own finalizers. A member may use a resource
/// this scope releases, so the order matters; the bound keeps `Drop` from
/// hanging on a body that ignores interruption.
const CLOSE_JOIN_BUDGET: std::time::Duration = std::time::Duration::from_secs(5);

/// How a scope saw its body end. `Unknown` is the honest answer for an
/// explicit [`Scope::close`] or a `Drop`, where no body exit exists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScopeOutcome {
    /// The body succeeded.
    Success,
    /// The body failed: typed failure, defect, interrupt, or a mix.
    Failure,
    /// The scope closed without observing a body exit.
    Unknown,
}

/// The exit a scope's finalizers can inspect, erased of the success value and
/// the typed error. `failures` counts typed `Fail` reasons; `defects` keeps
/// the unexpected failures concrete, so a release can report them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScopeExit {
    outcome: ScopeOutcome,
    failures: usize,
    interrupted: bool,
    defects: Vec<Defect>,
}

impl ScopeExit {
    /// A successful body exit.
    pub fn success() -> Self {
        Self {
            outcome: ScopeOutcome::Success,
            failures: 0,
            interrupted: false,
            defects: Vec::new(),
        }
    }

    /// No body exit to report (a direct close or a `Drop`).
    pub fn unknown() -> Self {
        Self {
            outcome: ScopeOutcome::Unknown,
            failures: 0,
            interrupted: false,
            defects: Vec::new(),
        }
    }

    /// Erase a finished [`Exit`]. Typed failures are counted, defects kept,
    /// and an interrupt recorded without discarding any other reason.
    pub fn from_exit<A, E>(exit: &Exit<A, E>) -> Self {
        match exit {
            Exit::Success(_) => Self::success(),
            Exit::Failure(cause) => Self::from_cause(cause),
        }
    }

    /// Erase a run result (what `Effect::scoped` holds at close time).
    pub fn from_result<A, E>(result: &Result<A, Cause<E>>) -> Self {
        match result {
            Ok(_) => Self::success(),
            Err(cause) => Self::from_cause(cause),
        }
    }

    fn from_cause<E>(cause: &Cause<E>) -> Self {
        Self {
            outcome: ScopeOutcome::Failure,
            failures: cause
                .reasons
                .iter()
                .filter(|reason| matches!(reason, Reason::Fail(_)))
                .count(),
            interrupted: cause
                .reasons
                .iter()
                .any(|reason| matches!(reason, Reason::Interrupt { .. })),
            defects: cause.defects().into_iter().cloned().collect(),
        }
    }

    /// The outcome shape.
    pub fn outcome(&self) -> ScopeOutcome {
        self.outcome
    }

    /// True only for [`ScopeOutcome::Success`].
    pub fn is_success(&self) -> bool {
        self.outcome == ScopeOutcome::Success
    }

    /// True for [`ScopeOutcome::Failure`].
    pub fn is_failure(&self) -> bool {
        self.outcome == ScopeOutcome::Failure
    }

    /// True when the body's cause carried an interrupt, even beside a failure.
    pub fn is_interrupted(&self) -> bool {
        self.interrupted
    }

    /// How many typed `Fail` reasons the body carried.
    pub fn failures(&self) -> usize {
        self.failures
    }

    /// Defects the body carried, in order.
    pub fn defects(&self) -> &[Defect] {
        &self.defects
    }
}

enum Finalizer {
    Sync(Box<dyn FnOnce(&ScopeExit) + Send>),
    Effect(Box<dyn FnOnce(&ScopeExit) -> Option<Defect> + Send>),
}

/// Run an effect finalizer, rendering failure to a [`Defect`].
fn render<E>(fin: impl FnOnce() -> Effect<(), E>) -> Option<Defect>
where
    E: Send + 'static + std::fmt::Debug,
{
    match fin().run_sync() {
        Ok(()) => None,
        Err(cause) => Some(render_once(cause)),
    }
}

/// Render a panicking finalizer as a [`Defect`]. A finalizer that unwinds must
/// not skip the rest of the stack: one bad release is a defect on the exit, and
/// the other releases still run (Scope.test.ts:107).
fn defect_from_panic(payload: Box<dyn std::any::Any + Send>) -> Defect {
    let message = payload
        .downcast_ref::<&str>()
        .map(|text| (*text).to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "non-string panic payload".to_string());
    Defect {
        message: format!("finalizer panicked: {message}"),
        backtrace: Backtrace::capture().to_string(),
    }
}

/// Render one failed finalizer cause as a [`Defect`].
fn render_once<E>(cause: Cause<E>) -> Defect
where
    E: Send + 'static + std::fmt::Debug,
{
    Defect {
        message: format!("finalizer failed: {cause:?}"),
        backtrace: Backtrace::capture().to_string(),
    }
}

/// The ambient scope service tag. `Effect::scoped` provides a fresh [`Scope`]
/// under this tag for the body it wraps, so an effect body reaches its own
/// scope with `use_service::<ScopeTag>()` instead of being handed one from
/// outside. The tag is the `R` side of `scoped`: `Effect<A, E, ScopeTag>`
/// becomes `Effect<A, E, ()>`.
pub struct ScopeTag;

impl crate::ServiceTag for ScopeTag {
    type Service = Arc<Scope>;
    const ID: &'static str = "elm-effect/scope";
}

/// A lifetime bound: closing interrupts member fibers, then runs finalizers
/// last-in-first-out, exactly once. Late registrations run inline.
pub struct Scope {
    id: u64,
    closed: AtomicBool,
    finalizers: Mutex<Vec<Finalizer>>,
    members: Mutex<Vec<Weak<FiberControl>>>,
    /// The exit this scope closed with, so a finalizer registered after close
    /// sees the same outcome. `None` before close and during the instant
    /// between claiming the close and recording it.
    close_exit: Mutex<Option<ScopeExit>>,
}

impl Scope {
    /// A standalone scope. Ids are caller-chosen labels; the runtime mints
    /// its own via [`crate::Runtime::create_scope`].
    pub fn new(id: u64) -> Self {
        Self {
            id,
            closed: AtomicBool::new(false),
            finalizers: Mutex::new(Vec::new()),
            members: Mutex::new(Vec::new()),
            close_exit: Mutex::new(None),
        }
    }

    pub fn id(&self) -> u64 {
        self.id
    }

    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    /// Lend this scope without closing it: `use_scope` runs with access,
    /// ownership stays with the caller.
    pub fn provide<T>(&self, use_scope: impl FnOnce(&Scope) -> T) -> T {
        use_scope(self)
    }

    /// A child scope: it closes when this scope closes, and closing it early
    /// releases only what it owns.
    ///
    /// The link is an ordinary finalizer registered now, so the child's close
    /// sits in the parent's LIFO order at the point it was forked: anything
    /// the parent registered later runs before the child, anything earlier
    /// runs after. A fork taken after the parent closed is born closed, which
    /// makes its registrations run inline rather than strand. This is the
    /// mechanism a failed layer build uses to release its own resources
    /// without closing the caller's scope.
    pub fn fork(&self) -> Arc<Scope> {
        let child = Arc::new(Scope::new(crate::fiber::next_id().0));
        let closing = Arc::clone(&child);
        self.add_finalizer(move || {
            closing.close();
        });
        child
    }

    /// Register a sync finalizer. Runs LIFO at close; never runs twice.
    /// Registration after close runs inline.
    pub fn add_finalizer(&self, f: impl FnOnce() + Send + 'static) {
        self.add_finalizer_with_exit(move |_| f());
    }

    /// Register a sync finalizer that sees how the scope's body ended. Same
    /// LIFO, once-only, and inline-after-close rules as
    /// [`add_finalizer`](Self::add_finalizer). The exit is
    /// [`ScopeExit::unknown`] when the scope is closed directly or by `Drop`,
    /// so a release that must act on a body outcome checks
    /// [`ScopeExit::is_failure`] rather than assuming success.
    pub fn add_finalizer_with_exit(&self, f: impl FnOnce(&ScopeExit) + Send + 'static) {
        let mut guard = self.finalizers.lock().unwrap();
        if self.closed.load(Ordering::SeqCst) {
            drop(guard);
            // A registration that races the close can land between the
            // closed flag and the recorded exit; `Unknown` is the honest
            // answer for that instant, not a guess at success.
            let exit = self
                .close_exit
                .lock()
                .unwrap()
                .clone()
                .unwrap_or_else(ScopeExit::unknown);
            f(&exit);
        } else {
            guard.push(Finalizer::Sync(Box::new(f)));
        }
    }

    /// Register an effect finalizer: the runtime runs it at close. A failed
    /// run renders to a [`Defect`] for the supervisor to append. Needs
    /// [`std::fmt::Debug`] on `E` to render the failure.
    ///
    /// Registration after close runs inline at once. A late run that fails
    /// returns its defect instead of dropping it: the caller surfaces it
    /// (return or append), never `let _`.
    pub fn add_finalizer_effect<E>(
        &self,
        fin: impl FnOnce() -> Effect<(), E> + Send + 'static,
    ) -> Option<Defect>
    where
        E: Send + 'static + std::fmt::Debug,
    {
        self.add_finalizer_effect_with_exit(move |_| fin())
    }

    /// Register an effect finalizer that sees how the scope's body ended.
    /// Same LIFO, once-only, masking, and inline-after-close rules as
    /// [`add_finalizer_effect`](Self::add_finalizer_effect). A late run sees
    /// the recorded exit (or [`ScopeExit::unknown`] in the instant before it
    /// is recorded) and returns its defect instead of dropping it.
    pub fn add_finalizer_effect_with_exit<E>(
        &self,
        fin: impl FnOnce(&ScopeExit) -> Effect<(), E> + Send + 'static,
    ) -> Option<Defect>
    where
        E: Send + 'static + std::fmt::Debug,
    {
        let mut guard = self.finalizers.lock().unwrap();
        if self.closed.load(Ordering::SeqCst) {
            drop(guard);
            let exit = self
                .close_exit
                .lock()
                .unwrap()
                .clone()
                .unwrap_or_else(ScopeExit::unknown);
            render(|| fin(&exit))
        } else {
            guard.push(Finalizer::Effect(Box::new(move |exit| {
                render(|| fin(exit))
            })));
            None
        }
    }

    /// Bracket acquisition to this scope: `acquire` runs now (the caller is
    /// already inside the runtime), and `release` is registered as an effect
    /// finalizer, so success, typed failure, and interruption all release
    /// (L6). The resource is shared by clone between caller and finalizer:
    /// wrap non-cloneable resources in an [`Arc`] and release through it.
    /// Closed-scope edge: the resource releases inline and the caller sees
    /// interruption instead of a silent success.
    pub fn acquire_release<A, E, FRel, ERel>(
        &self,
        acquire: Effect<A, E>,
        release: FRel,
    ) -> Result<A, Cause<E>>
    where
        A: Send + 'static + Clone,
        E: Send + 'static,
        FRel: FnOnce(A) -> Effect<(), ERel> + Send + 'static,
        ERel: Send + 'static + std::fmt::Debug,
    {
        // residual-790: `?` form, not match — clippy::collapsible_match clean.
        let resource = acquire.run_sync()?;
        // Closed before or during acquisition: release inline now and report
        // interruption instead of a silent success (nothing may leak into a
        // dead scope).
        if self.is_closed() {
            return match release(resource).run_sync() {
                Ok(()) => Err(Cause::interrupt(None)),
                Err(cause) => {
                    let mut out = Cause::interrupt(None);
                    out.reasons.push(Reason::Die(render_once(cause)));
                    Err(out)
                }
            };
        }
        let give = resource.clone();
        let late = self.add_finalizer_effect(move || release(resource));
        // Race window: close() ran between the check and the push. The push
        // ran the release inline, so report the interruption it signalled,
        // carrying a late-release defect instead of dropping it.
        if self.is_closed() {
            // The registered clone already released; drop the spare.
            drop(give);
            let mut out = Cause::interrupt(None);
            if let Some(defect) = late {
                out.reasons.push(Reason::Die(defect));
            }
            return Err(out);
        }
        Ok(give)
    }

    pub(crate) fn add_member(&self, fiber: &Arc<FiberControl>) {
        self.members.lock().unwrap().push(Arc::downgrade(fiber));
    }

    /// Close once without a body exit. Finalizers registered with
    /// [`add_finalizer_with_exit`](Self::add_finalizer_with_exit) see
    /// [`ScopeExit::unknown`]. Prefer [`close_with`](Self::close_with) when
    /// the caller has the body's outcome.
    pub fn close(&self) -> Vec<Defect> {
        self.close_with(&ScopeExit::unknown())
    }

    /// Close once with the body's exit: interrupt members first, then run
    /// finalizers LIFO. Returns effect-finalizer failures for the supervisor
    /// to append. Late registrations run inline with the same accounting.
    ///
    /// Masks the current thread's fiber itself (L7): direct close, `Drop`,
    /// layer-failure, and dispose paths all run finalizers uninterruptibly,
    /// not just the runtime fiber-completion path. Off-fiber threads run
    /// sync finalizers to completion anyway, so there is nothing to mask.
    pub fn close_with(&self, exit: &ScopeExit) -> Vec<Defect> {
        if self.closed.swap(true, Ordering::SeqCst) {
            return Vec::new();
        }
        *self.close_exit.lock().unwrap() = Some(exit.clone());
        match FiberControl::with_current_masked(|| self.close_inner(exit)) {
            Some(spills) => spills,
            None => self.close_inner(exit),
        }
    }

    fn close_inner(&self, exit: &ScopeExit) -> Vec<Defect> {
        let members = std::mem::take(&mut *self.members.lock().unwrap());
        let closing = FiberControl::current_id();
        let mut live = Vec::new();
        for weak in members {
            if let Some(fiber) = weak.upgrade() {
                // A deferred member only observes the interrupt after its
                // start latch fires; waiting without this would burn the
                // whole budget on a fiber that never began.
                fiber.start.fire();
                fiber.request_interrupt();
                // A member may close the scope it belongs to; waiting for it
                // would wait for the close to return.
                if Some(fiber.id) != closing {
                    live.push(fiber);
                }
            }
        }
        // Wait for the interrupted members before releasing the scope's own
        // resources: a member's release may still need them. Effect 4 waits
        // without a bound; the bound here is the price of a blocking Drop.
        let deadline = std::time::Instant::now() + CLOSE_JOIN_BUDGET;
        let mut stragglers = Vec::new();
        for fiber in live {
            if !fiber.wait_done_until(deadline) {
                stragglers.push(fiber.id);
            }
        }
        let finalizers = std::mem::take(&mut *self.finalizers.lock().unwrap());
        let mut spills = Vec::new();
        // A member still running at the deadline means the release order was
        // not guaranteed; report it instead of pretending the close ordered it.
        for id in stragglers {
            spills.push(Defect {
                message: format!("scope close timed out waiting for member fiber {id:?}"),
                backtrace: Backtrace::capture().to_string(),
            });
        }
        for fin in finalizers.into_iter().rev() {
            match fin {
                Finalizer::Sync(run) => {
                    if let Err(payload) =
                        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(exit)))
                    {
                        spills.push(defect_from_panic(payload));
                    }
                }
                Finalizer::Effect(run) => {
                    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(exit))) {
                        Ok(None) => {}
                        Ok(Some(defect)) => spills.push(defect),
                        Err(payload) => spills.push(defect_from_panic(payload)),
                    }
                }
            }
        }
        spills
    }
}

impl Drop for Scope {
    fn drop(&mut self) {
        let _ = self.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn close_runs_finalizers_lifo_once() {
        let scope = Scope::new(1);
        let order = Arc::new(Mutex::new(Vec::new()));
        for i in 0..3 {
            let order = order.clone();
            scope.add_finalizer(move || order.lock().unwrap().push(i));
        }
        assert!(scope.close().is_empty());
        assert!(scope.close().is_empty());
        assert_eq!(*order.lock().unwrap(), vec![2, 1, 0]);

        // Late finalizers run immediately, still once.
        let count = Arc::new(AtomicUsize::new(0));
        let c = count.clone();
        scope.add_finalizer(move || {
            c.fetch_add(1, Ordering::SeqCst);
        });
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn effect_finalizer_failure_is_returned_not_thrown() {
        let scope = Scope::new(2);
        scope.add_finalizer_effect(|| Effect::fail("rel".to_string()));
        let spills = scope.close();
        assert_eq!(spills.len(), 1);
        assert!(spills[0].message.contains("rel"));
    }

    #[test]
    fn a_throwing_finalizer_does_not_skip_the_rest() {
        let scope = Scope::new(9);
        let ran = Arc::new(Mutex::new(Vec::new()));
        let remaining = Arc::clone(&ran);
        scope.add_finalizer(move || remaining.lock().unwrap().push("remaining"));
        scope.add_finalizer(|| panic!("release"));

        let spills = scope.close();
        assert_eq!(
            *ran.lock().unwrap(),
            vec!["remaining"],
            "a throwing finalizer must not skip the one registered before it"
        );
        assert_eq!(spills.len(), 1);
        assert!(
            spills[0].message.contains("release"),
            "the panic rides the close as a defect: {:?}",
            spills[0].message
        );
    }

    #[test]
    fn provide_lends_without_closing() {
        let scope = Scope::new(3);
        let id = scope.provide(|s| s.id());
        assert_eq!(id, 3);
        assert!(!scope.is_closed());
    }

    #[test]
    fn close_masks_current_fiber_while_finalizers_run() {
        use crate::fiber::{FiberControl, StartLatch};
        use std::sync::atomic::AtomicBool;
        let control = Arc::new(FiberControl::new(
            crate::FiberId(999),
            None,
            Vec::new(),
            false,
            Arc::new(StartLatch::default()),
        ));
        let _enter = control.enter_current();
        let scope = Scope::new(41);
        let seen_masked = Arc::new(AtomicBool::new(false));
        let probe = seen_masked.clone();
        let control_in_fin = Arc::clone(&control);
        scope.add_finalizer(move || {
            probe.store(control_in_fin.masked_now(), Ordering::SeqCst);
        });
        assert!(scope.close().is_empty());
        assert!(
            seen_masked.load(Ordering::SeqCst),
            "finalizers must run with the current fiber masked"
        );
        assert!(
            !control.masked_now(),
            "mask depth must return to 0 after close"
        );
    }

    #[test]
    fn late_effect_finalizer_defect_is_returned_not_dropped() {
        let scope = Scope::new(7);
        assert!(scope.close().is_empty());
        let defect = scope.add_finalizer_effect(|| Effect::fail("late".to_string()));
        let defect = defect.expect("late effect-finalizer failure must surface, never drop");
        assert!(defect.message.contains("late"));
    }
}
