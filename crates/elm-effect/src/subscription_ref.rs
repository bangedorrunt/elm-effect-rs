//! `SubscriptionRef.ts` port: shared cell with change streams.
//!
//! A [`SubscriptionRef`] holds one current value. [`get`](SubscriptionRef::get)
//! reads it, [`set`](SubscriptionRef::set)/[`update`](SubscriptionRef::update)
//! replace it, [`get_and_update`](SubscriptionRef::get_and_update) returns
//! the previous value. [`subscribe`](SubscriptionRef::subscribe) opens a
//! [`ChangeStream`] that first replays the current value, then yields each
//! later update in order; [`unsubscribe`](ChangeStream::unsubscribe) stops
//! delivery (the explicit form of interruption here). Concurrent
//! subscribe/unsubscribe races resolve through the single hub lock, so no
//! update is delivered twice or skipped for a live stream.

use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex};

struct Hub<A> {
    current: Mutex<A>,
    streams: Mutex<Vec<std::sync::Weak<StreamInner<A>>>>,
}

struct StreamInner<A> {
    queue: Mutex<VecDeque<A>>,
    wake: Condvar,
    live: Mutex<bool>,
}

/// Live change stream: current value first, then updates.
pub struct ChangeStream<A> {
    inner: Arc<StreamInner<A>>,
}

impl<A: Clone> ChangeStream<A> {
    /// Blocking next change, or [`None`] after unsubscribe.
    pub fn next(&self) -> Option<A> {
        let mut guard = self.inner.queue.lock().unwrap();
        loop {
            if let Some(v) = guard.pop_front() {
                return Some(v);
            }
            if !*self.inner.live.lock().unwrap() {
                return None;
            }
            guard = self.inner.wake.wait(guard).unwrap();
        }
    }

    /// Non-blocking next change.
    pub fn poll(&self) -> Option<A> {
        self.inner.queue.lock().unwrap().pop_front()
    }

    /// Stop delivery; pending [`next`](ChangeStream::next) resolves [`None`].
    pub fn unsubscribe(&self) {
        *self.inner.live.lock().unwrap() = false;
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
            hub: Arc::new(Hub { current: Mutex::new(initial), streams: Mutex::new(Vec::new()) }),
        }
    }

    /// Type guard (`isSubscriptionRef`).
    pub fn is_subscription_ref(&self) -> bool {
        true
    }

    /// Current value.
    pub fn get(&self) -> A {
        self.hub.current.lock().unwrap().clone()
    }

    /// Replace, notifying every live stream.
    pub fn set(&self, value: A) {
        *self.hub.current.lock().unwrap() = value.clone();
        self.notify(&value);
    }

    /// Replace through `f`, returning the new value. Atomic: the cell
    /// lock is held across `f`, so `f` must not reenter the cell.
    pub fn update(&self, f: impl FnOnce(A) -> A) -> A {
        let mut guard = self.hub.current.lock().unwrap();
        let next = f(guard.clone());
        *guard = next.clone();
        drop(guard);
        self.notify(&next);
        next
    }

    /// Replace through `f`, returning the previous value (atomic as above).
    pub fn get_and_update(&self, f: impl FnOnce(A) -> A) -> A {
        let mut guard = self.hub.current.lock().unwrap();
        let prev = guard.clone();
        let next = f(prev.clone());
        *guard = next.clone();
        drop(guard);
        self.notify(&next);
        prev
    }

    /// Update only when `f` returns [`Some`], always returning the
    /// current value afterwards.
    pub fn update_some(&self, f: impl FnOnce(A) -> Option<A>) -> A {
        let mut guard = self.hub.current.lock().unwrap();
        let prev = guard.clone();
        match f(prev.clone()) {
            Some(next) => {
                *guard = next.clone();
                drop(guard);
                self.notify(&next);
                next
            }
            None => prev,
        }
    }

    /// Open a stream: current value first, then each later update.
    pub fn subscribe(&self) -> ChangeStream<A> {
        let stream = Arc::new(StreamInner {
            queue: Mutex::new(VecDeque::from([self.get()])),
            wake: Condvar::new(),
            live: Mutex::new(true),
        });
        self.hub.streams.lock().unwrap().push(Arc::downgrade(&stream));
        ChangeStream { inner: stream }
    }

    fn notify(&self, value: &A) {
        let mut guard = self.hub.streams.lock().unwrap();
        guard.retain(|w| match w.upgrade() {
            Some(s) => {
                if *s.live.lock().unwrap() {
                    s.queue.lock().unwrap().push_back(value.clone());
                    s.wake.notify_all();
                    true
                } else {
                    false
                }
            }
            None => false,
        });
    }
}
