//! `PubSub.ts` port: broadcast with per-subscriber queues.
//!
//! A [`PubSub`] fans each published value out to every live subscription,
//! each with its own bounded queue. [`Strategy`] picks the full-queue
//! policy: [`Strategy::Bounded`] parks the publisher until space frees,
//! [`Strategy::Dropping`] drops the newest value, [`Strategy::Sliding`]
//! evicts the oldest, [`Strategy::Unbounded`] never fills. Publishing with
//! no subscribers drops the values (size stays 0). [`PubSub::shutdown`]
//! wakes every blocked [`take`](Subscription::take) with [`None`].
//! [`subscribe_replay`](PubSub::subscribe_replay) pre-fills a new
//! subscription from the trailing history window.
//!
//! Threads stand in for fibers here (blocking park/unpark via one
//! [`Condvar`](std::sync::Condvar) per subscription); Effect scopes and
//! interruption rows are thread residuals, not silent skips.
// This file belongs to the parked port campaign: a test reaches it through
// a `#[path]` include to port specific rows, so most of its surface is
// unused by any one test target. Warnings about the unexercised remainder
// describe the test's coverage, not a defect in the crate, so they are
// allowed here rather than fixed one by one.
#![allow(dead_code)]
use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex};

/// Full-queue policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Strategy {
    /// Block the publisher until space frees.
    Bounded,
    /// Drop the incoming value when full.
    Dropping,
    /// Evict the oldest value when full.
    Sliding,
    /// No bound (capacity ignored).
    Unbounded,
}

/// Live subscription: one private queue plus a wake signal.
pub struct Subscription<A> {
    queue: Mutex<VecDeque<A>>,
    wake: Condvar,
    live: Mutex<bool>,
    parent: std::sync::Weak<PubSubInner<A>>,
}

impl<A> Subscription<A> {
    /// Blocking take: the oldest value, or [`None`] after shutdown or
    /// unsubscribe.
    pub fn take(&self) -> Option<A> {
        let mut guard = self.queue.lock().unwrap();
        loop {
            if let Some(v) = guard.pop_front() {
                self.wake.notify_all();
                drop(guard);
                self.note_drained();
                return Some(v);
            }
            if !*self.live.lock().unwrap() || self.parent_shut() {
                return None;
            }
            guard = self.wake.wait(guard).unwrap();
        }
    }

    /// Non-blocking take.
    pub fn poll(&self) -> Option<A> {
        let out = self.queue.lock().unwrap().pop_front();
        if out.is_some() {
            self.wake.notify_all();
            self.note_drained();
        }
        out
    }

    /// Take up to `n` values (fractional counts floor, `NaN` takes none).
    pub fn take_up_to(&self, n: f64) -> Vec<A> {
        if n.is_nan() || n < 1.0 {
            return Vec::new();
        }
        self.take_n(n as usize)
    }

    /// Non-blocking take of up to `n` values (same count rules).
    pub fn poll_up_to(&self, n: f64) -> Vec<A> {
        if n.is_nan() || n < 1.0 {
            return Vec::new();
        }
        self.poll_n(n as usize)
    }

    /// Take up to `n` values, blocking for at least one.
    pub fn take_n(&self, n: usize) -> Vec<A> {
        let mut out = Vec::new();
        if n == 0 {
            return out;
        }
        if let Some(first) = self.take() {
            out.push(first);
        } else {
            return out;
        }
        out.extend(self.poll_n(n - 1));
        out
    }

    /// Non-blocking drain of up to `n`.
    pub fn poll_n(&self, n: usize) -> Vec<A> {
        let mut guard = self.queue.lock().unwrap();
        let mut out = Vec::new();
        while out.len() < n {
            match guard.pop_front() {
                Some(v) => out.push(v),
                None => break,
            }
        }
        if !out.is_empty() {
            self.wake.notify_all();
            drop(guard);
            self.note_drained();
        }
        out
    }

    /// Drain everything waiting.
    pub fn take_all(&self) -> Vec<A> {
        let mut guard = self.queue.lock().unwrap();
        let out: Vec<A> = guard.drain(..).collect();
        if !out.is_empty() {
            self.wake.notify_all();
            drop(guard);
            self.note_drained();
        }
        out
    }

    /// Take between `min` and `max`, blocking until at least `min` (or end).
    pub fn take_between(&self, min: usize, max: usize) -> Vec<A> {
        let mut out = Vec::new();
        while out.len() < min {
            match self.take() {
                Some(v) => out.push(v),
                None => return out,
            }
        }
        out.extend(self.poll_n(max.saturating_sub(out.len())));
        out
    }

    /// Detach from the hub; pending takes resolve [`None`].
    pub fn unsubscribe(&self) {
        *self.live.lock().unwrap() = false;
        self.wake.notify_all();
    }

    fn parent_shut(&self) -> bool {
        self.parent.upgrade().is_none_or(|p| p.is_shutdown())
    }

    fn note_drained(&self) {
        if let Some(p) = self.parent.upgrade() {
            p.wake_all();
        }
    }
}

struct PubSubInner<A> {
    capacity: usize,
    strategy: Strategy,
    replay: usize,
    subs: Mutex<Vec<std::sync::Weak<Subscription<A>>>>,
    history: Mutex<VecDeque<A>>,
    shutdown: Mutex<bool>,
    space: Condvar,
}

impl<A> PubSubInner<A> {
    fn is_shutdown(&self) -> bool {
        *self.shutdown.lock().unwrap()
    }

    fn wake_all(&self) {
        self.space.notify_all();
        for sub in self
            .subs
            .lock()
            .unwrap()
            .iter()
            .filter_map(std::sync::Weak::upgrade)
        {
            sub.wake.notify_all();
        }
    }

    fn live_subs(&self) -> Vec<Arc<Subscription<A>>> {
        let mut guard = self.subs.lock().unwrap();
        let mut live = Vec::new();
        guard.retain(|w| match w.upgrade() {
            Some(s) if *s.live.lock().unwrap() => {
                live.push(s);
                true
            }
            _ => false,
        });
        live
    }
}

/// Broadcast hub. Clone shares the hub.
#[derive(Clone)]
pub struct PubSub<A> {
    inner: Arc<PubSubInner<A>>,
}

impl<A: Clone> PubSub<A> {
    fn make(capacity: usize, strategy: Strategy, replay: usize) -> Self {
        Self {
            inner: Arc::new(PubSubInner {
                capacity,
                strategy,
                replay,
                subs: Mutex::new(Vec::new()),
                history: Mutex::new(VecDeque::new()),
                shutdown: Mutex::new(false),
                space: Condvar::new(),
            }),
        }
    }

    /// Bounded hub: publishers park while any queue is full.
    pub fn bounded(capacity: usize) -> Self {
        Self::make(capacity.max(1), Strategy::Bounded, 0)
    }

    /// Dropping hub: full queues drop incoming values.
    pub fn dropping(capacity: usize) -> Self {
        Self::make(capacity.max(1), Strategy::Dropping, 0)
    }

    /// Sliding hub: full queues evict the oldest value.
    pub fn sliding(capacity: usize) -> Self {
        Self::make(capacity.max(1), Strategy::Sliding, 0)
    }

    /// Unbounded hub.
    pub fn unbounded() -> Self {
        Self::make(usize::MAX, Strategy::Unbounded, 0)
    }

    /// Hub with a trailing replay window for late subscribers.
    pub fn with_replay(capacity: usize, strategy: Strategy, replay: usize) -> Self {
        Self::make(capacity.max(1), strategy, replay)
    }

    /// Type guard: every value of this type is a hub (`isPubSub`).
    pub fn is_pubsub(&self) -> bool {
        true
    }

    /// Total queued values across live subscribers.
    pub fn size(&self) -> usize {
        self.inner
            .live_subs()
            .iter()
            .map(|s| s.queue.lock().unwrap().len())
            .sum()
    }

    /// True when nothing is queued anywhere.
    pub fn is_empty(&self) -> bool {
        self.size() == 0
    }

    /// True when any live queue reached capacity (unbounded never is).
    pub fn is_full(&self) -> bool {
        if self.inner.strategy == Strategy::Unbounded {
            return false;
        }
        self.inner
            .live_subs()
            .iter()
            .any(|s| s.queue.lock().unwrap().len() >= self.inner.capacity)
    }

    /// True after [`PubSub::shutdown`].
    pub fn is_shutdown(&self) -> bool {
        self.inner.is_shutdown()
    }

    /// Shut down: pending and future takes resolve [`None`].
    pub fn shutdown(&self) {
        *self.inner.shutdown.lock().unwrap() = true;
        self.inner.wake_all();
    }

    /// Publish one value. `false` only after shutdown; with no
    /// subscribers the value is dropped and size stays 0.
    pub fn publish(&self, value: A) -> bool {
        if self.is_shutdown() {
            return false;
        }
        if self.inner.strategy == Strategy::Bounded {
            let mut guard = self.inner.subs.lock().unwrap();
            loop {
                if self.is_shutdown() {
                    return false;
                }
                let mut full = false;
                for weak in guard.iter() {
                    if let Some(sub) = weak.upgrade()
                        && *sub.live.lock().unwrap()
                        && sub.queue.lock().unwrap().len() >= self.inner.capacity
                    {
                        full = true;
                        break;
                    }
                }
                if !full {
                    break;
                }
                guard = self.inner.space.wait(guard).unwrap();
            }
        }
        let subs = self.inner.live_subs();
        for sub in &subs {
            let mut q = sub.queue.lock().unwrap();
            match self.inner.strategy {
                Strategy::Bounded | Strategy::Unbounded => {
                    q.push_back(value.clone());
                }
                Strategy::Dropping => {
                    if q.len() < self.inner.capacity {
                        q.push_back(value.clone());
                    }
                }
                Strategy::Sliding => {
                    if q.len() >= self.inner.capacity {
                        q.pop_front();
                    }
                    q.push_back(value.clone());
                }
            }
            sub.wake.notify_all();
        }
        if self.inner.replay > 0 {
            let mut hist = self.inner.history.lock().unwrap();
            hist.push_back(value);
            while hist.len() > self.inner.replay {
                hist.pop_front();
            }
        }
        true
    }

    /// Publish many in order.
    pub fn publish_all(&self, values: impl IntoIterator<Item = A>) -> bool {
        let mut ok = true;
        for v in values {
            ok = self.publish(v) && ok;
        }
        ok
    }

    /// Subscribe with a fractional replay count: non-positive and `NaN`
    /// disable replay, anything else rounds up.
    pub fn subscribe_replay_count(&self, n: f64) -> Arc<Subscription<A>> {
        if n.is_nan() || n <= 0.0 {
            return self.subscribe_replay(0);
        }
        self.subscribe_replay(n.ceil() as usize)
    }

    /// Subscribe with an empty queue.
    pub fn subscribe(&self) -> Arc<Subscription<A>> {
        self.subscribe_replay(0)
    }

    /// Subscribe pre-filled with up to `n` trailing values.
    pub fn subscribe_replay(&self, n: usize) -> Arc<Subscription<A>> {
        let hist: Vec<A> = {
            let h = self.inner.history.lock().unwrap();
            h.iter().rev().take(n).rev().cloned().collect()
        };
        let sub = Arc::new(Subscription {
            queue: Mutex::new(hist.into()),
            wake: Condvar::new(),
            live: Mutex::new(true),
            parent: Arc::downgrade(&self.inner),
        });
        self.inner.subs.lock().unwrap().push(Arc::downgrade(&sub));
        sub
    }

    /// Snapshot of the trailing history window.
    pub fn replay_window(&self) -> Vec<A> {
        self.inner.history.lock().unwrap().iter().cloned().collect()
    }
}
