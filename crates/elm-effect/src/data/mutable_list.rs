//! `MutableList.ts` port: an owned, interior-mutable list.
//!
//! Single-threaded interior mutability (`RefCell`); sharing across threads
//! wants `MutableRef`/`Queue` instead. Residuals: iterator invalidation
//! rules (leases end at each call here — documented, no aliasing).

use std::cell::RefCell;

/// A list mutated in place.
#[derive(Clone, Debug, Default)]
pub struct MutableList<T> {
    inner: RefCell<Vec<T>>,
}

impl<T> MutableList<T> {
    /// Empty list (`empty`, `make` with no elements).
    pub fn new() -> Self {
        Self {
            inner: RefCell::new(Vec::new()),
        }
    }

    /// From initial elements (`make`, `fromIterable`).
    pub fn from_vec(values: Vec<T>) -> Self {
        Self {
            inner: RefCell::new(values),
        }
    }

    /// Push at the back (`append`, `push`).
    pub fn push(&self, value: T) {
        self.inner.borrow_mut().push(value);
    }

    /// Push many, returning how many were added (`appendAll`).
    pub fn append_all(&self, values: impl IntoIterator<Item = T>) -> usize {
        let mut guard = self.inner.borrow_mut();
        let before = guard.len();
        guard.extend(values);
        guard.len() - before
    }

    /// Prepend many, preserving order (`prependAll`).
    pub fn prepend_all(&self, values: impl IntoIterator<Item = T>) {
        let mut guard = self.inner.borrow_mut();
        let mut front: Vec<T> = values.into_iter().collect();
        front.append(&mut guard);
        *guard = front;
    }

    /// Take up to `n` from the front, advancing past them (`takeN`).
    pub fn take_n(&self, n: usize) -> Vec<T> {
        let mut guard = self.inner.borrow_mut();
        let at = n.min(guard.len());
        guard.drain(..at).collect()
    }

    /// Keep matching values in place (`filter`).
    pub fn filter(&self, mut predicate: impl FnMut(&T) -> bool) {
        self.inner.borrow_mut().retain(|v| predicate(v));
    }

    /// Pop from the back (`pop`).
    pub fn pop(&self) -> Option<T> {
        self.inner.borrow_mut().pop()
    }

    /// Get a clone by index (`get`).
    pub fn get(&self, index: usize) -> Option<T>
    where
        T: Clone,
    {
        self.inner.borrow().get(index).cloned()
    }

    /// Overwrite by index; `false` when out of bounds (`set`).
    pub fn set(&self, index: usize, value: T) -> bool {
        match self.inner.borrow_mut().get_mut(index) {
            Some(slot) => {
                *slot = value;
                true
            }
            None => false,
        }
    }

    /// Element count (`size`).
    pub fn len(&self) -> usize {
        self.inner.borrow().len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.borrow().is_empty()
    }

    /// Drop all elements (`clear`).
    pub fn clear(&self) {
        self.inner.borrow_mut().clear();
    }

    /// Snapshot to a vector (`toArray`).
    pub fn to_vec(&self) -> Vec<T>
    where
        T: Clone,
    {
        self.inner.borrow().clone()
    }

    /// Remove by index (`remove`); `None` when out of bounds.
    pub fn remove(&self, index: usize) -> Option<T> {
        let mut guard = self.inner.borrow_mut();
        if index < guard.len() {
            Some(guard.remove(index))
        } else {
            None
        }
    }
}
