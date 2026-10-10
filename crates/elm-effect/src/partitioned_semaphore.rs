//! `PartitionedSemaphore.ts` port: one semaphore per key.
//!
//! A [`PartitionedSemaphore`] keeps an independent permit pool behind
//! every key. [`with_permits`](PartitionedSemaphore::with_permits) runs a
//! closure while holding `n` permits of one key and releases on return,
//! including unwinds (the [`Guard`] releases on drop, which is how a
//! cancelled waiter frees partially acquired permits here).
//! [`try_acquire`](PartitionedSemaphore::try_acquire) never blocks and
//! reports availability. Different keys never contend: acquisition reads
//! only its own key's pool under the single hub lock, so competing takes
//! on distinct keys proceed independently and a resumed waiter finds its
//! own key's permits untouched.
//!
//! Fibers and cross-schedule interruption belong to the Effect runtime;
//! the explicit [`Guard`] drop is the portable cancellation story.
// This file belongs to the parked port campaign: a test reaches it through
// a `#[path]` include to port specific rows, so most of its surface is
// unused by any one test target. Warnings about the unexercised remainder
// describe the test's coverage, not a defect in the crate, so they are
// allowed here rather than fixed one by one.
#![allow(dead_code)]
use std::collections::HashMap;
use std::hash::Hash;
use std::sync::{Arc, Condvar, Mutex};

/// Scoped permit hold: releasing happens on drop, including unwinds.
pub struct Guard {
    release: Option<Box<dyn FnOnce() + Send>>,
}

impl Drop for Guard {
    fn drop(&mut self) {
        if let Some(release) = self.release.take() {
            release();
        }
    }
}

struct Hub<K> {
    permits: usize,
    states: Mutex<HashMap<K, usize>>,
    wake: Condvar,
}

/// Per-key semaphore pool. Clone shares the pools.
#[derive(Clone)]
pub struct PartitionedSemaphore<K> {
    hub: Arc<Hub<K>>,
}

impl<K: Eq + Hash + Clone + Send + Sync + 'static> PartitionedSemaphore<K> {
    /// Build with `permits` per key.
    pub fn new(permits: usize) -> Self {
        Self {
            hub: Arc::new(Hub { permits, states: Mutex::new(HashMap::new()), wake: Condvar::new() }),
        }
    }

    /// Instance and module APIs agree (`module-level combinators delegate
    /// to the instance api`).
    pub fn permits_per_key(&self) -> usize {
        self.hub.permits
    }

    /// Block until `need` permits of `key` arrive; zero runs immediately.
    pub fn acquire(&self, key: K, need: usize) -> Guard {
        if need == 0 {
            return Guard { release: None };
        }
        let mut states = self.hub.states.lock().unwrap();
        loop {
            let available = states.get(&key).copied().unwrap_or(self.hub.permits);
            if available >= need {
                states.insert(key.clone(), available - need);
                drop(states);
                return self.guard_for(key, need);
            }
            states = self.hub.wake.wait(states).unwrap();
        }
    }

    fn guard_for(&self, key: K, need: usize) -> Guard {
        let hub = self.hub.clone();
        Guard {
            release: Some(Box::new(move || {
                let mut states = hub.states.lock().unwrap();
                let available = states.get(&key).copied().unwrap_or(hub.permits);
                states.insert(key.clone(), (available + need).min(hub.permits));
                hub.wake.notify_all();
            })),
        }
    }

    /// Run `f` while holding `need` permits of `key`.
    pub fn with_permits<R>(&self, key: K, need: usize, f: impl FnOnce() -> R) -> R {
        let _guard = self.acquire(key, need);
        f()
    }

    /// Try without blocking: some guard when available, none otherwise.
    pub fn try_acquire(&self, key: K, need: usize) -> Option<Guard> {
        if self.try_take(&key, need) {
            Some(self.guard_for(key, need))
        } else {
            None
        }
    }

    fn try_take(&self, key: &K, need: usize) -> bool {
        if need == 0 {
            return true;
        }
        let mut states = self.hub.states.lock().unwrap();
        let available = states.get(key).copied().unwrap_or(self.hub.permits);
        if available >= need {
            states.insert(key.clone(), available - need);
            true
        } else {
            false
        }
    }

    /// Available permits snapshot for one key.
    pub fn available(&self, key: K) -> usize {
        self.hub.states.lock().unwrap().get(&key).copied().unwrap_or(self.hub.permits)
    }
}

impl<K: Eq + Hash + Clone> Hub<K> {
    fn release(&self, key: &K, permits: usize) {
        let mut states = self.states.lock().unwrap();
        let available = states.get(key).copied().unwrap_or(self.permits);
        states.insert(key.clone(), (available + permits).min(self.permits));
        self.wake.notify_all();
    }
}
