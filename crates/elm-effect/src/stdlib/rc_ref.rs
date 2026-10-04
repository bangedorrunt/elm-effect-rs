//! `RcRef` port (`RcRef.ts` at the pin): reference-counted cached value.
//!
//! [`get`](RcRef::get) shares one acquired value while references are out;
//! the value releases when the last reference returns AND the entry is
//! idle past [`idle_ttl`](RcRef::set_idle_ttl) (no TTL releases
//! immediately). [`touch`](RcRef::touch) resets the idle clock;
//! [`invalidate`](RcRef::invalidate) releases now.

use std::sync::Mutex;
use std::time::{Duration, Instant};

struct State<T> {
    cached: Option<T>,
    refs: usize,
    idle_since: Option<Instant>,
}

/// Reference-counted cached acquisition with an explicit idle policy.
pub struct RcRef<T> {
    acquire: Box<dyn Fn() -> T + Send>,
    release: Box<dyn Fn(T) + Send>,
    idle_ttl: Mutex<Option<Duration>>,
    state: Mutex<State<T>>,
}

impl<T> RcRef<T> {
    /// Build from acquire/release closures (`make`). No idle TTL: the value
    /// releases as soon as the last reference returns.
    pub fn make(
        acquire: impl Fn() -> T + Send + 'static,
        release: impl Fn(T) + Send + 'static,
    ) -> Self {
        Self {
            acquire: Box::new(acquire),
            release: Box::new(release),
            idle_ttl: Mutex::new(None),
            state: Mutex::new(State {
                cached: None,
                refs: 0,
                idle_since: None,
            }),
        }
    }

    /// Set the idle time-to-live: a zero-reference value older than this is
    /// released on next [`get`](RcRef::get) instead of reused.
    pub fn set_idle_ttl(&self, ttl: Option<Duration>) {
        *self.idle_ttl.lock().expect("rcref") = ttl;
    }

    fn expired(&self, since: Instant) -> bool {
        match *self.idle_ttl.lock().expect("rcref") {
            None => true,
            Some(ttl) => since.elapsed() >= ttl,
        }
    }

    /// Shared value, acquiring (or re-acquiring past TTL) as needed.
    /// Values handed out are clones; the releasable original stays cached.
    pub fn get(&self) -> T
    where
        T: Clone,
    {
        let mut guard = self.state.lock().expect("rcref");
        let stale = match (&guard.cached, guard.idle_since) {
            (Some(_), Some(since)) if guard.refs == 0 => self.expired(since),
            (None, _) => true,
            _ => false,
        };
        if stale {
            if let Some(old) = guard.cached.take() {
                (self.release)(old);
            }
            guard.idle_since = None;
        }
        if guard.cached.is_none() {
            guard.cached = Some((self.acquire)());
        }
        guard.refs += 1;
        guard.cached.as_ref().expect("just acquired").clone()
    }

    /// Return one reference. At zero references the idle clock starts (or
    /// the value releases immediately without a TTL).
    pub fn release_ref(&self) {
        let mut guard = self.state.lock().expect("rcref");
        guard.refs = guard.refs.saturating_sub(1);
        if guard.refs == 0 {
            match *self.idle_ttl.lock().expect("rcref") {
                None => {
                    if let Some(old) = guard.cached.take() {
                        (self.release)(old);
                    }
                    guard.idle_since = None;
                }
                Some(_) => {
                    guard.idle_since = Some(Instant::now());
                }
            }
        }
    }

    /// Outstanding reference count.
    pub fn ref_count(&self) -> usize {
        self.state.lock().expect("rcref").refs
    }

    /// `true` while a value is cached.
    pub fn is_acquired(&self) -> bool {
        self.state.lock().expect("rcref").cached.is_some()
    }

    /// Reset the idle clock; `false` when nothing is cached.
    pub fn touch(&self) -> bool {
        let mut guard = self.state.lock().expect("rcref");
        if guard.cached.is_some() {
            guard.idle_since = Some(Instant::now());
            true
        } else {
            false
        }
    }

    /// Release the cached value now (use when unreferenced).
    pub fn invalidate(&self) {
        let mut guard = self.state.lock().expect("rcref");
        if let Some(old) = guard.cached.take() {
            (self.release)(old);
        }
        guard.refs = 0;
        guard.idle_since = None;
    }
}

impl<T> Drop for RcRef<T> {
    fn drop(&mut self) {
        if let Some(old) = self.state.lock().expect("rcref").cached.take() {
            (self.release)(old);
        }
    }
}
