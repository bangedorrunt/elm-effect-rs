//! `FiberSet` port (`FiberSet.ts` at the pin): supervised fibers.
//!
//! A set owns a [`Scope`](crate::Scope): every fiber runs bound to it, so
//! closing the set interrupts the members. [`join`](FiberSet::join) collects
//! the members (first failure wins, like the tag's supervised join);
//! [`await_empty`](FiberSet::await_empty) waits for quiescence and reaps;
//! [`clear`](FiberSet::clear) interrupts everything and drains.
//!
//! Residuals: attempt telemetry and scope-replacement hooks.
// This file belongs to the parked port campaign: a test reaches it through
// a `#[path]` include to port specific rows, so most of its surface is
// unused by any one test target. Warnings about the unexercised remainder
// describe the test's coverage, not a defect in the crate, so they are
// allowed here rather than fixed one by one.
#![allow(dead_code)]
use elm_effect::{Ctx, Deferred, Exit, FiberHandle, ForkOpts, Runtime, Scope};
use std::sync::{Arc, Mutex};

type Shared<A, E> = Arc<Mutex<Option<FiberHandle<A, E>>>>;
/// Shared so a [`FiberSetRuntime`] forked from the set tracks into the same
/// list the set reads.
type Members<A, E> = Arc<Mutex<Vec<Shared<A, E>>>>;
/// The set's `deferred`: completed when an external interruption reaches a
/// member that propagates interruption. Its payload is unit because no
/// carried row reads more than whether it settled.
type Settled = Arc<Deferred<(), ()>>;

/// A supervised set of fibers sharing one scope.
pub struct FiberSet<A: Send + 'static, E: Send + 'static> {
    // `Arc`, not a clone: a `Runtime` clone would shut the shared runtime
    // down on drop, and a set's runtime outlives the set's own handle.
    rt: Arc<Runtime>,
    scope: Arc<Scope>,
    members: Members<A, E>,
    settled: Settled,
}

impl<A, E> FiberSet<A, E>
where
    A: Send + 'static,
    E: Send + 'static,
{
    /// Empty set on a fresh scope.
    pub fn new(rt: Runtime) -> Self {
        Self {
            scope: rt.create_scope(),
            rt: Arc::new(rt),
            members: Arc::new(Mutex::new(Vec::new())),
            settled: Arc::new(Deferred::new()),
        }
    }

    /// The set's scope. Closing it interrupts every member, which is the
    /// scoped row's shape (`Effect.scoped` closing around the set).
    pub fn scope(&self) -> Arc<Scope> {
        Arc::clone(&self.scope)
    }

    /// `FiberSet.toJSON`: the `_id` discriminant identifying this value.
    pub fn to_json(&self) -> FiberSetTag {
        FiberSetTag { id: "FiberSet" }
    }

    /// Fork `body` into the set's scope and track it. Starts immediately,
    /// which is the pin's default.
    pub fn run(&self, body: impl FnOnce(&Ctx) -> Exit<A, E> + Send + 'static) {
        self.fork(body, ForkOpts::immediate(), None, false);
    }

    /// Fork without starting: the body runs when the set is next yielded to
    /// (a join, an interrupt, or shutdown fires the start latch).
    pub fn run_deferred(&self, body: impl FnOnce(&Ctx) -> Exit<A, E> + Send + 'static) {
        self.fork(body, ForkOpts::deferred(), None, false);
    }

    /// Fork with an interruption observer attached to the member's own
    /// finalizers, which is what `Effect.onInterrupt` contributes in the pin.
    pub fn run_observed(
        &self,
        body: impl FnOnce(&Ctx) -> Exit<A, E> + Send + 'static,
        on_interrupt: impl FnOnce() + Send + 'static,
    ) -> usize {
        self.fork(body, ForkOpts::immediate(), Some(Box::new(on_interrupt)), false)
    }

    /// Fork a member that settles the set when it ends interrupted
    /// (`propagateInterruption`). Returns the member's position, which is the
    /// fiber `FiberSet.run` hands back in the pin.
    pub fn run_propagating(
        &self,
        body: impl FnOnce(&Ctx) -> Exit<A, E> + Send + 'static,
        propagate: bool,
    ) -> usize {
        self.fork(body, ForkOpts::immediate(), None, propagate)
    }

    /// `true` once a propagating member ended interrupted.
    pub fn is_settled(&self) -> bool {
        self.settled.is_done()
    }

    fn fork(
        &self,
        body: impl FnOnce(&Ctx) -> Exit<A, E> + Send + 'static,
        opts: ForkOpts,
        observer: Option<Box<dyn FnOnce() + Send>>,
        propagate: bool,
    ) -> usize {
        let settled = Arc::clone(&self.settled);
        let handle = self.rt.fork_scoped(
            &self.scope,
            move |ctx| {
                let exit = body(ctx);
                if propagate && matches!(&exit, Exit::Failure(cause) if cause.has_interrupts()) {
                    settled.complete(());
                }
                exit
            },
            opts,
        );
        if let Some(observer) = observer {
            handle.add_finalizer(observer);
        }
        let mut members = self.members.lock().expect("set lock");
        members.push(Arc::new(Mutex::new(Some(handle))));
        members.len() - 1
    }

    /// Tracked member count.
    pub fn size(&self) -> usize {
        self.members.lock().expect("set lock").len()
    }

    /// `true` with no tracked members.
    pub fn is_empty(&self) -> bool {
        self.size() == 0
    }

    fn take_all(&mut self) -> Vec<FiberHandle<A, E>> {
        let mut members = self.members.lock().expect("set lock");
        let taken = std::mem::take(&mut *members);
        drop(members);
        taken
            .into_iter()
            .filter_map(|shared| shared.lock().expect("set lock").take())
            .collect()
    }

    /// Interrupt one member by position, waiting for its finalizers, which is
    /// the pin's `Fiber.interrupt(fiber)` on the fiber `run` returned.
    pub fn interrupt_member(&self, index: usize) {
        let shared = self.members.lock().expect("set lock").get(index).cloned();
        if let Some(shared) = shared {
            if let Some(handle) = shared.lock().expect("set lock").as_ref() {
                handle.interrupt();
            }
        }
    }

    /// Join every member, draining the set. First failure wins; success
    /// carries `()`.
    pub fn join(&mut self) -> Exit<(), E> {
        let mut first_failure = None;
        for handle in self.take_all() {
            match handle.join() {
                Exit::Success(_) => {}
                Exit::Failure(cause) => {
                    if first_failure.is_none() {
                        first_failure = Some(cause);
                    }
                }
            }
        }
        match first_failure {
            Some(cause) => Exit::Failure(cause),
            None => Exit::Success(()),
        }
    }

    /// Wait until every member finishes, then reap. Returns like [`join`](FiberSet::join).
    pub fn await_empty(&mut self) -> Exit<(), E> {
        loop {
            let done = self.members.lock().expect("set lock").iter().all(|shared| {
                shared
                    .lock()
                    .expect("set lock")
                    .as_ref()
                    .is_none_or(|h| h.is_done())
            });
            if done {
                break;
            }
            std::thread::park_timeout(std::time::Duration::from_millis(1));
        }
        self.join()
    }

    /// Interrupt every member without draining: a later [`join`](FiberSet::join)
    /// records the interruption as the set's failure (external interruption
    /// is observed, not swallowed). `clear` drains; this keeps the exits.
    pub fn interrupt_all(&self) {
        for shared in self.members.lock().expect("set lock").iter() {
            if let Some(handle) = shared.lock().expect("set lock").as_ref() {
                handle.interrupt();
            }
        }
    }

    /// Interrupt every member and drain the set.
    pub fn clear(&mut self) {
        for shared in self.members.lock().expect("set lock").iter() {
            if let Some(handle) = shared.lock().expect("set lock").as_ref() {
                handle.interrupt();
            }
        }
        for handle in self.take_all() {
            let _ = handle.join();
        }
    }

    /// `FiberSet.runtime`: a fork function bound to this set, callable from
    /// outside an effect, tracking into the same member list.
    pub fn runtime(&self) -> FiberSetRuntime<A, E> {
        FiberSetRuntime {
            rt: Arc::clone(&self.rt),
            scope: Arc::clone(&self.scope),
            members: Arc::clone(&self.members),
        }
    }

    /// `FiberSet.makeRuntimePromise`: the promise's resolve is the exit, its
    /// reject the failing exit. Callable outside an effect, which is the
    /// point of the row.
    pub fn runtime_promise(body: impl FnOnce(&Ctx) -> Exit<A, E> + Send + 'static) -> Exit<A, E> {
        Runtime::new().fork_detach(body).join()
    }
}

/// `FiberSet.toJSON()`'s shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FiberSetTag {
    /// The discriminant: `"FiberSet"`.
    pub id: &'static str,
}

/// The function `FiberSet.runtime` returns: fork into the set it came from.
#[derive(Clone)]
pub struct FiberSetRuntime<A: Send + 'static, E: Send + 'static> {
    rt: Arc<Runtime>,
    scope: Arc<Scope>,
    members: Members<A, E>,
}

impl<A, E> FiberSetRuntime<A, E>
where
    A: Send + 'static,
    E: Send + 'static,
{
    /// Fork `body` into the set this runtime belongs to.
    pub fn run(&self, body: impl FnOnce(&Ctx) -> Exit<A, E> + Send + 'static) {
        self.fork(body, None);
    }

    /// Fork with an interruption observer, like [`FiberSet::run_observed`].
    pub fn run_observed(
        &self,
        body: impl FnOnce(&Ctx) -> Exit<A, E> + Send + 'static,
        on_interrupt: impl FnOnce() + Send + 'static,
    ) {
        self.fork(body, Some(Box::new(on_interrupt)));
    }

    fn fork(
        &self,
        body: impl FnOnce(&Ctx) -> Exit<A, E> + Send + 'static,
        observer: Option<Box<dyn FnOnce() + Send>>,
    ) {
        let handle = self.rt.fork_scoped(&self.scope, body, ForkOpts::immediate());
        if let Some(observer) = observer {
            handle.add_finalizer(observer);
        }
        self.members
            .lock()
            .expect("set lock")
            .push(Arc::new(Mutex::new(Some(handle))));
    }
}
