//! `Resource` port (`Resource.ts` at the pin): managed acquire/release.
//!
//! A resource caches one acquired value: [`get`](Resource::get) acquires on
//! first use, [`refresh`](Resource::refresh) releases and re-acquires,
//! [`invalidate`](Resource::invalidate) releases and forgets. Values are
//! released on scope drop semantics held by the owner: dropping a
//! [`Resource`] releases its cached value, so nothing leaks past the owner.
//!
//! Residuals: effect-scoped acquisition (acquire-inside-scope finalizers
//! live in `Scope`, owned elsewhere), auto-refresh policies.

/// Acquire-once, refreshable, drop-releasing value holder.
pub struct Resource<T> {
    acquire: Box<dyn Fn() -> T + Send>,
    release: Box<dyn Fn(T) + Send>,
    current: std::sync::Mutex<Option<T>>,
}

impl<T> Resource<T> {
    /// Build from acquire/release closures (`manual` shape).
    pub fn manual(
        acquire: impl Fn() -> T + Send + 'static,
        release: impl Fn(T) + Send + 'static,
    ) -> Self {
        Self {
            acquire: Box::new(acquire),
            release: Box::new(release),
            current: std::sync::Mutex::new(None),
        }
    }

    /// Cached value, acquiring on first use. Requires `Clone` to hand out
    /// shared copies while retaining the releasable original.
    pub fn get(&self) -> T
    where
        T: Clone,
    {
        let mut guard = self.current.lock().expect("resource");
        if guard.is_none() {
            *guard = Some((self.acquire)());
        }
        guard.as_ref().expect("just acquired").clone()
    }

    /// Release the cached value and re-acquire on next [`get`](Resource::get).
    pub fn invalidate(&self) {
        if let Some(value) = self.current.lock().expect("resource").take() {
            (self.release)(value);
        }
    }

    /// Release and immediately re-acquire, returning the fresh value.
    pub fn refresh(&self) -> T
    where
        T: Clone,
    {
        self.invalidate();
        self.get()
    }

    /// `true` when a value is currently cached.
    pub fn is_acquired(&self) -> bool {
        self.current.lock().expect("resource").is_some()
    }

    /// Any resource counts (TS `isResource`; its negative half is a foreign
    /// value, which cannot reach this predicate, so only the positive case
    /// has a Rust spelling).
    pub fn is_resource(&self) -> bool {
        true
    }
}

impl<T> Drop for Resource<T> {
    fn drop(&mut self) {
        if let Some(value) = self.current.lock().expect("resource").take() {
            (self.release)(value);
        }
    }
}
