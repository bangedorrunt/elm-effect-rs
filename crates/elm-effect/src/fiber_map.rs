//! `FiberMap` port (`FiberMap.ts` at the pin): keyed supervised fibers.
//!
//! Same supervision as [`FiberSet`](super::fiber_set::FiberSet), keyed by
//! `K`: [`set`](FiberMap::set) replaces (interrupting the previous fiber),
//! [`remove`](FiberMap::remove) interrupts one member, [`join`](FiberMap::join)
//! collects all with first-failure-wins.
//!
//! Residuals: `runtimePromise`, per-key join handles, attempt telemetry.

use elm_effect::{Ctx, Exit, FiberHandle, ForkOpts, Runtime, Scope};
use std::collections::HashMap;
use std::hash::Hash;
use std::sync::{Arc, Mutex};

type Shared<A, E> = Arc<Mutex<Option<FiberHandle<A, E>>>>;

/// Supervised fibers keyed by `K`.
pub struct FiberMap<K, A: Send + 'static, E: Send + 'static> {
    rt: Runtime,
    scope: Arc<Scope>,
    members: HashMap<K, Shared<A, E>>,
}

impl<K, A, E> FiberMap<K, A, E>
where
    K: Hash + Eq + Clone,
    A: Send + 'static,
    E: Send + 'static,
{
    /// Empty map on a fresh scope.
    pub fn new(rt: Runtime) -> Self {
        Self {
            scope: rt.create_scope(),
            rt,
            members: HashMap::new(),
        }
    }

    /// Fork `body` under `key`, interrupting any previous member there.
    pub fn set(&mut self, key: K, body: impl FnOnce(&Ctx) -> Exit<A, E> + Send + 'static) {
        if let Some(old) = self.members.remove(&key)
            && let Some(handle) = old.lock().expect("map lock").take()
        {
            handle.interrupt();
            let _ = handle.join();
        }
        let handle = self
            .rt
            .fork_scoped(&self.scope, body, ForkOpts::immediate());
        self.members.insert(key, Arc::new(Mutex::new(Some(handle))));
    }

    /// `true` when `key` holds a member.
    pub fn has(&self, key: &K) -> bool {
        self.members.contains_key(key)
    }

    /// Interrupt and drop the member at `key`; `false` when absent.
    pub fn remove(&mut self, key: &K) -> bool {
        match self.members.remove(key) {
            None => false,
            Some(shared) => {
                if let Some(handle) = shared.lock().expect("map lock").take() {
                    handle.interrupt();
                    let _ = handle.join();
                }
                true
            }
        }
    }

    /// Tracked member count.
    pub fn size(&self) -> usize {
        self.members.len()
    }

    /// Join every member and drain. First failure wins.
    pub fn join(&mut self) -> Exit<(), E> {
        let members = std::mem::take(&mut self.members);
        let mut first_failure = None;
        for shared in members.values() {
            if let Some(handle) = shared.lock().expect("map lock").take()
                && let Exit::Failure(cause) = handle.join()
                && first_failure.is_none()
            {
                first_failure = Some(cause);
            }
        }
        match first_failure {
            Some(cause) => Exit::Failure(cause),
            None => Exit::Success(()),
        }
    }

    /// Interrupt every member and drain.
    pub fn clear(&mut self) {
        for shared in self.members.values() {
            if let Some(handle) = shared.lock().expect("map lock").as_ref() {
                handle.interrupt();
            }
        }
        let members = std::mem::take(&mut self.members);
        for shared in members.values() {
            if let Some(handle) = shared.lock().expect("map lock").take() {
                let _ = handle.join();
            }
        }
    }
}
