//! Mutable values scoped to a fiber.
//!
//! Unlike [`Reference`](crate::Reference), which is read-only configuration
//! overridden for a sub-effect, a `FiberRef` can be changed while a fiber runs.
//! A child inherits the parent's value at fork; later changes in the child stay
//! in the child. [`locally`](FiberRef::locally) installs a value for one region
//! and restores the previous value when the region exits.

use std::convert::Infallible;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::{Effect, Requirement};

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// A mutable, per-fiber value with a default.
///
/// Cloning the handle shares the identity and default; the value itself stays
/// on each fiber.
pub struct FiberRef<A: Clone + Send + Sync + 'static> {
    id: u64,
    default: A,
}

impl<A: Clone + Send + Sync + 'static> Clone for FiberRef<A> {
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            default: self.default.clone(),
        }
    }
}

impl<A: Clone + Send + Sync + 'static> FiberRef<A> {
    /// Make a reference with `default` as the value on every fiber that has
    /// not set it.
    pub fn make(default: A) -> Self {
        Self {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            default,
        }
    }

    /// Read the current fiber's value, or the default when it was never set.
    ///
    /// Asking outside a fiber is a defect, because there is no fiber whose
    /// value could be read. Every runtime edge runs the effect on a fiber.
    pub fn get(&self) -> Effect<A, Infallible, ()> {
        let id = self.id;
        let default = self.default.clone();
        Effect::with_fiber(move |fiber| {
            let fiber = fiber.clone();
            Effect::<A, Infallible, ()>::sync(move || fiber.fiber_ref_get(id, &default))
        })
    }

    /// Replace the current fiber's value.
    ///
    /// Asking outside a fiber is a defect, because there is no fiber whose
    /// value could be changed.
    pub fn set(&self, value: A) -> Effect<(), Infallible, ()> {
        let id = self.id;
        Effect::with_fiber(move |fiber| {
            let fiber = fiber.clone();
            Effect::<(), Infallible, ()>::sync(move || fiber.fiber_ref_set(id, value))
        })
    }

    /// Replace the current fiber's value through `f`.
    ///
    /// Asking outside a fiber is a defect, because there is no fiber whose
    /// value could be changed.
    pub fn update<F>(&self, f: F) -> Effect<(), Infallible, ()>
    where
        F: FnOnce(A) -> A + Send + 'static,
    {
        let id = self.id;
        let default = self.default.clone();
        Effect::with_fiber(move |fiber| {
            let fiber = fiber.clone();
            Effect::<(), Infallible, ()>::sync(move || {
                let next = f(fiber.fiber_ref_get(id, &default));
                fiber.fiber_ref_set(id, next);
            })
        })
    }

    /// Replace the current fiber's value through `f`, returning `f`'s first
    /// component. The second component is the new stored value.
    ///
    /// Asking outside a fiber is a defect, because there is no fiber whose
    /// value could be changed.
    pub fn modify<B, F>(&self, f: F) -> Effect<B, Infallible, ()>
    where
        B: Send + 'static,
        F: FnOnce(A) -> (B, A) + Send + 'static,
    {
        let id = self.id;
        let default = self.default.clone();
        Effect::with_fiber(move |fiber| {
            let fiber = fiber.clone();
            Effect::<B, Infallible, ()>::sync(move || {
                let (out, next) = f(fiber.fiber_ref_get(id, &default));
                fiber.fiber_ref_set(id, next);
                out
            })
        })
    }

    /// Run `effect` with this reference set to `value`, restoring the previous
    /// value when the effect exits. A child forked inside the region inherits
    /// `value`; its later changes remain local to that child.
    pub fn locally<B, E, R>(&self, value: A, effect: Effect<B, E, R>) -> Effect<B, E, R>
    where
        B: Send + 'static,
        E: Send + 'static,
        R: Requirement,
    {
        let id = self.id;
        Effect::with_fiber(move |fiber| {
            let fiber = fiber.clone();
            let previous: Option<A> = fiber.fiber_ref_get_option(id);
            let previous = std::sync::Arc::new(std::sync::Mutex::new(Some(previous)));
            fiber.fiber_ref_set(id, value);
            effect.ensuring(move || {
                let previous = previous.lock().unwrap().take();
                if let Some(previous) = previous {
                    fiber.fiber_ref_restore(id, previous);
                }
            })
        })
    }
}
