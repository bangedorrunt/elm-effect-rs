//! `ScopedRef` port (`ScopedRef.ts` at the pin): a settable shared cell.
//!
//! [`ScopedRef::make`] builds from an initial evaluation; [`set`](ScopedRef::set)
//! replaces the value; [`get`](ScopedRef::get) clones it out.

use std::sync::{Arc, Mutex};

/// Shared mutable cell with eager initial value.
#[derive(Clone, Debug)]
pub struct ScopedRef<T> {
    backing: Arc<Mutex<T>>,
}

impl<T> ScopedRef<T> {
    /// Build by evaluating once now (`make`).
    pub fn make(evaluate: impl FnOnce() -> T) -> Self {
        Self {
            backing: Arc::new(Mutex::new(evaluate())),
        }
    }

    /// Build from an acquire closure (`fromAcquire` shape).
    pub fn from_acquire(acquire: impl FnOnce() -> T) -> Self {
        Self::make(acquire)
    }

    /// Current value, cloned out.
    pub fn get(&self) -> T
    where
        T: Clone,
    {
        self.backing.lock().expect("scoped ref").clone()
    }

    /// Replace the value.
    pub fn set(&self, value: T) {
        *self.backing.lock().expect("scoped ref") = value;
    }
}
