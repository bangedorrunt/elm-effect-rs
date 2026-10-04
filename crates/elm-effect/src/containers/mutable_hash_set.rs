//! Mutable `HashSet` port (`MutableHashSet.ts` at the pin).
//!
//! In-place set behind shared ownership; same clone-out rule as
//! [`super::mutable_hash_map`].

use std::cell::RefCell;
use std::collections::HashSet as StdSet;
use std::hash::Hash;
use std::rc::Rc;

/// In-place set behind shared ownership.
#[derive(Clone, Debug, Default)]
pub struct MutableHashSet<T> {
    inner: Rc<RefCell<StdSet<T>>>,
}

impl<T> MutableHashSet<T>
where
    T: Hash + Eq,
{
    /// Empty set.
    pub fn empty() -> Self {
        Self {
            inner: Rc::new(RefCell::new(StdSet::new())),
        }
    }

    /// Set of the given values.
    pub fn make(values: impl IntoIterator<Item = T>) -> Self {
        Self {
            inner: Rc::new(RefCell::new(values.into_iter().collect())),
        }
    }

    /// `true` when `value` is present.
    pub fn has(&self, value: &T) -> bool {
        self.inner.borrow().contains(value)
    }

    /// Add `value` in place; `true` when newly added.
    pub fn add(&self, value: T) -> bool {
        self.inner.borrow_mut().insert(value)
    }

    /// Remove `value` in place; `true` when present.
    pub fn remove(&self, value: &T) -> bool {
        self.inner.borrow_mut().remove(value)
    }

    /// Number of values.
    pub fn size(&self) -> usize {
        self.inner.borrow().len()
    }

    /// `true` with no values.
    pub fn is_empty(&self) -> bool {
        self.inner.borrow().is_empty()
    }

    /// Snapshot of all values.
    pub fn values(&self) -> Vec<T>
    where
        T: Clone,
    {
        self.inner.borrow().iter().cloned().collect()
    }
}
