//! `SubscriptionRef.ts` port: shared cell with change streams.
//!
//! A [`SubscriptionRef`] holds one current value. [`get`](SubscriptionRef::get)
//! reads it, [`set`](SubscriptionRef::set)/[`update`](SubscriptionRef::update)
//! replace it, [`get_and_update`](SubscriptionRef::get_and_update) returns
//! the previous value. [`subscribe`](SubscriptionRef::subscribe) opens a
//! [`ChangeStream`] that first replays the current value, then yields each
//! later update in order; [`unsubscribe`](ChangeStream::unsubscribe) stops
//! delivery (the explicit form of interruption here). Subscribe snapshots the
//! current value and registers under the same hub lock `set` takes, so an
//! update that lands concurrently either predates the snapshot or is queued
//! for the new stream: it cannot fall between the two.
//!
//! A stream's queue is unbounded, like the pin's `Queue.unbounded()`: a slow
//! consumer holds every update rather than losing one.

use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex};

/// Lock a cell even after a panic poisoned it. The guarded state here is a
/// plain value or a one-shot slot, and a panic in a user callback leaves it
/// unchanged rather than half-written, so recovery beats bricking the cell.
trait RecoverLock<T> {
    fn recover(&self) -> std::sync::MutexGuard<'_, T>;
}

impl<T> RecoverLock<T> for std::sync::Mutex<T> {
    fn recover(&self) -> std::sync::MutexGuard<'_, T> {
        self.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

struct HubState<A> {
    current: A,
    streams: Vec<std::sync::Weak<StreamInner<A>>>,
}

struct Hub<A> {
    state: Mutex<HubState<A>>,
}

struct StreamState<A> {
    queue: VecDeque<A>,
    live: bool,
}

struct StreamInner<A> {
    state: Mutex<StreamState<A>>,
    wake: Condvar,
}

/// Live change stream: current value first, then updates.
pub struct ChangeStream<A> {
    inner: Arc<StreamInner<A>>,
}

impl<A: Clone> ChangeStream<A> {
    /// Blocking next change, or [`None`] after unsubscribe.
    pub fn next(&self) -> Option<A> {
        let mut guard = self.inner.state.recover();
        loop {
            if let Some(v) = guard.queue.pop_front() {
                return Some(v);
            }
            if !guard.live {
                return None;
            }
            guard = self.inner.wake.wait(guard).unwrap();
        }
    }

    /// Non-blocking next change.
    pub fn poll(&self) -> Option<A> {
        self.inner.state.recover().queue.pop_front()
    }

    /// Stop delivery; pending [`next`](ChangeStream::next) resolves [`None`].
    pub fn unsubscribe(&self) {
        self.inner.state.recover().live = false;
        self.inner.wake.notify_all();
    }
}

/// Shared cell with streams (`SubscriptionRef`). Clone shares the cell.
#[derive(Clone)]
pub struct SubscriptionRef<A> {
    hub: Arc<Hub<A>>,
}

impl<A: Clone> SubscriptionRef<A> {
    /// Build holding `initial`.
    pub fn new(initial: A) -> Self {
        Self {
            hub: Arc::new(Hub {
                state: Mutex::new(HubState {
                    current: initial,
                    streams: Vec::new(),
                }),
            }),
        }
    }

    /// Type guard (`isSubscriptionRef`).
    pub fn is_subscription_ref(&self) -> bool {
        true
    }

    /// Current value.
    pub fn get(&self) -> A {
        self.hub.state.recover().current.clone()
    }

    /// Replace, notifying every live stream.
    pub fn set(&self, value: A) {
        let mut state = self.hub.state.recover();
        state.current = value.clone();
        Self::notify(&mut state, &value);
    }

    /// Replace through `f`, returning the new value. Atomic: the cell
    /// lock is held across `f`, so `f` must not reenter the cell.
    pub fn update(&self, f: impl FnOnce(A) -> A) -> A {
        let mut state = self.hub.state.recover();
        let next = f(state.current.clone());
        state.current = next.clone();
        Self::notify(&mut state, &next);
        next
    }

    /// Replace through `f`, returning the previous value. Atomic like
    /// [`update`](SubscriptionRef::update): the lock is held across `f`, so
    /// `f` must not reenter the cell.
    pub fn get_and_update(&self, f: impl FnOnce(A) -> A) -> A {
        let mut state = self.hub.state.recover();
        let prev = state.current.clone();
        let next = f(prev.clone());
        state.current = next.clone();
        Self::notify(&mut state, &next);
        prev
    }

    /// Update only when `f` returns [`Some`], always returning the
    /// current value afterwards. Atomic like [`update`](SubscriptionRef::update):
    /// the lock is held across `f`, so `f` must not reenter the cell.
    pub fn update_some(&self, f: impl FnOnce(A) -> Option<A>) -> A {
        let mut state = self.hub.state.recover();
        let prev = state.current.clone();
        match f(prev.clone()) {
            Some(next) => {
                state.current = next.clone();
                Self::notify(&mut state, &next);
                next
            }
            None => prev,
        }
    }

    /// Open a stream: current value first, then each later update. The
    /// snapshot and the registration share the hub lock with `set`, so a
    /// concurrent update is either in the snapshot or queued after it.
    pub fn subscribe(&self) -> ChangeStream<A> {
        let mut hub = self.hub.state.recover();
        let stream = Arc::new(StreamInner {
            state: Mutex::new(StreamState {
                queue: VecDeque::from([hub.current.clone()]),
                live: true,
            }),
            wake: Condvar::new(),
        });
        hub.streams.push(Arc::downgrade(&stream));
        ChangeStream { inner: stream }
    }

    fn notify(state: &mut HubState<A>, value: &A) {
        state.streams.retain(|w| match w.upgrade() {
            Some(s) => {
                let mut stream = s.state.recover();
                if stream.live {
                    stream.queue.push_back(value.clone());
                    s.wake.notify_all();
                }
                stream.live
            }
            None => false,
        });
    }
}
