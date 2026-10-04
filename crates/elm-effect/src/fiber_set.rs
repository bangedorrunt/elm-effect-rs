//! `FiberSet` port (`FiberSet.ts` at the pin): supervised fibers.
//!
//! A set owns a [`Scope`](crate::Scope): every fiber runs bound to it, so
//! closing the set interrupts the members. [`join`](FiberSet::join) collects
//! the members (first failure wins, like the tag's supervised join);
//! [`await_empty`](FiberSet::await_empty) waits for quiescence and reaps;
//! [`clear`](FiberSet::clear) interrupts everything and drains.
//!
//! Residuals: `runtimePromise` (needs the runtime-as-effect shape), attempt
//! telemetry, and scope-replacement hooks.

use elm_effect::{Ctx, Exit, FiberHandle, ForkOpts, Runtime, Scope};
use std::sync::{Arc, Mutex};

type Shared<A, E> = Arc<Mutex<Option<FiberHandle<A, E>>>>;

/// A supervised set of fibers sharing one scope.
pub struct FiberSet<A: Send + 'static, E: Send + 'static> {
    rt: Runtime,
    scope: Arc<Scope>,
    members: Vec<Shared<A, E>>,
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
            rt,
            members: Vec::new(),
        }
    }

    /// Fork `body` into the set's scope and track it.
    pub fn run(&mut self, body: impl FnOnce(&Ctx) -> Exit<A, E> + Send + 'static) {
        let handle = self
            .rt
            .fork_scoped(&self.scope, body, ForkOpts::immediate());
        self.members.push(Arc::new(Mutex::new(Some(handle))));
    }

    /// Tracked member count.
    pub fn size(&self) -> usize {
        self.members.len()
    }

    /// `true` with no tracked members.
    pub fn is_empty(&self) -> bool {
        self.members.is_empty()
    }

    fn take_all(&mut self) -> Vec<FiberHandle<A, E>> {
        let members = std::mem::take(&mut self.members);
        members
            .into_iter()
            .filter_map(|shared| shared.lock().expect("set lock").take())
            .collect()
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
            let done = self.members.iter().all(|shared| {
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

    /// Interrupt every member and drain the set.
    pub fn clear(&mut self) {
        for shared in &self.members {
            if let Some(handle) = shared.lock().expect("set lock").as_ref() {
                handle.interrupt();
            }
        }
        for handle in self.take_all() {
            let _ = handle.join();
        }
    }
}
