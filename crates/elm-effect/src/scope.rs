//! A [`Scope`] bounds fiber lifetimes and owns finalizers.
//!
//! Wave-1 (.4) mechanism: an id, a run-once LIFO finalizer stack, and member
//! interruption on close. The `acquire_release` policy and every-exit-path
//! wiring land in the Scope bead (.5), extending this type in place.

use std::sync::{
    Arc, Mutex, Weak,
    atomic::{AtomicBool, Ordering},
};

use crate::fiber::FiberControl;

/// A lifetime bound: closing interrupts member fibers, then runs finalizers
/// last-in-first-out, exactly once.
pub struct Scope {
    id: u64,
    closed: AtomicBool,
    finalizers: Mutex<Vec<Box<dyn FnOnce() + Send>>>,
    members: Mutex<Vec<Weak<FiberControl>>>,
}

impl Scope {
    pub(crate) fn new(id: u64) -> Self {
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

    /// Register a finalizer. Runs LIFO at close; never runs twice.
    /// Registration after close runs inline, still once.
    pub fn add_finalizer(&self, f: impl FnOnce() + Send + 'static) {
        let mut guard = self.finalizers.lock().unwrap();
        if self.closed.load(Ordering::SeqCst) {
            drop(guard);
            f();
        } else {
            guard.push(Box::new(f));
        }
    }

    pub(crate) fn add_member(&self, fiber: &Arc<FiberControl>) {
        self.members.lock().unwrap().push(Arc::downgrade(fiber));
    }

    /// Close once: interrupt members first, then run finalizers LIFO.
    /// Late finalizers (added after close) run immediately at registration.
    pub fn close(&self) {
        if self.closed.swap(true, Ordering::SeqCst) {
            return;
        }
        let members = std::mem::take(&mut *self.members.lock().unwrap());
        for weak in members {
            if let Some(fiber) = weak.upgrade() {
                fiber.request_interrupt();
            }
        }
        let finalizers = std::mem::take(&mut *self.finalizers.lock().unwrap());
        for run in finalizers.into_iter().rev() {
            run();
        }
    }
}

impl Drop for Scope {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn close_runs_finalizers_lifo_once() {
        let scope = Scope::new(1);
        let order = Arc::new(Mutex::new(Vec::new()));
        for i in 0..3 {
            let order = order.clone();
            scope.add_finalizer(move || order.lock().unwrap().push(i));
        }
        scope.close();
        scope.close();
        assert_eq!(*order.lock().unwrap(), vec![2, 1, 0]);

        // Late finalizers run immediately, still once.
        let count = Arc::new(AtomicUsize::new(0));
        let c = count.clone();
        scope.add_finalizer(move || {
            c.fetch_add(1, Ordering::SeqCst);
        });
        // Registration after close runs it inline.
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }
}
