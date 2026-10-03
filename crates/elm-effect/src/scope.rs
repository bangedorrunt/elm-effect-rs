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
//! `Drop` closes the scope, so it is the release path only for resources
//! whose finalizers need no `.await`: everything here runs synchronously.

use std::backtrace::Backtrace;
use std::sync::{
    Arc, Mutex, Weak,
    atomic::{AtomicBool, Ordering},
};

use crate::fiber::FiberControl;
use crate::{Cause, Defect, Effect, Reason};

enum Finalizer {
    Sync(Box<dyn FnOnce() + Send>),
    Effect(Box<dyn FnOnce() -> Option<Defect> + Send>),
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

/// A lifetime bound: closing interrupts member fibers, then runs finalizers
/// last-in-first-out, exactly once. Late registrations run inline.
pub struct Scope {
    id: u64,
    closed: AtomicBool,
    finalizers: Mutex<Vec<Finalizer>>,
    members: Mutex<Vec<Weak<FiberControl>>>,
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

    /// Register a sync finalizer. Runs LIFO at close; never runs twice.
    /// Registration after close runs inline.
    pub fn add_finalizer(&self, f: impl FnOnce() + Send + 'static) {
        let mut guard = self.finalizers.lock().unwrap();
        if self.closed.load(Ordering::SeqCst) {
            drop(guard);
            f();
        } else {
            guard.push(Finalizer::Sync(Box::new(f)));
        }
    }

    /// Register an effect finalizer: the runtime runs it at close. A failed
    /// run renders to a [`Defect`] for the supervisor to append. Needs
    /// [`std::fmt::Debug`] on `E` to render the failure.
    pub fn add_finalizer_effect<E>(&self, fin: impl FnOnce() -> Effect<(), E> + Send + 'static)
    where
        E: Send + 'static + std::fmt::Debug,
    {
        let mut guard = self.finalizers.lock().unwrap();
        if self.closed.load(Ordering::SeqCst) {
            drop(guard);
            let _ = render(fin);
        } else {
            guard.push(Finalizer::Effect(Box::new(move || render(fin))));
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
        let resource = match acquire.run_sync() {
            Ok(a) => a,
            Err(cause) => return Err(cause),
        };
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
        self.add_finalizer_effect(move || release(resource));
        // Race window: close() ran between the check and the push. The push
        // ran the release inline, so report the interruption it signalled.
        if self.is_closed() {
            // The registered clone already released; drop the spare.
            drop(give);
            return Err(Cause::interrupt(None));
        }
        Ok(give)
    }

    pub(crate) fn add_member(&self, fiber: &Arc<FiberControl>) {
        self.members.lock().unwrap().push(Arc::downgrade(fiber));
    }

    /// Close once: interrupt members first, then run finalizers LIFO.
    /// Returns effect-finalizer failures for the supervisor to append.
    /// Late registrations run inline with the same accounting.
    pub fn close(&self) -> Vec<Defect> {
        if self.closed.swap(true, Ordering::SeqCst) {
            return Vec::new();
        }
        let members = std::mem::take(&mut *self.members.lock().unwrap());
        for weak in members {
            if let Some(fiber) = weak.upgrade() {
                fiber.request_interrupt();
            }
        }
        let finalizers = std::mem::take(&mut *self.finalizers.lock().unwrap());
        let mut spills = Vec::new();
        for fin in finalizers.into_iter().rev() {
            match fin {
                Finalizer::Sync(run) => run(),
                Finalizer::Effect(run) => {
                    if let Some(defect) = run() {
                        spills.push(defect);
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
    fn provide_lends_without_closing() {
        let scope = Scope::new(3);
        let id = scope.provide(|s| s.id());
        assert_eq!(id, 3);
        assert!(!scope.is_closed());
    }
}
