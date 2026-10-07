//! `Differ.ts` port: diffs as values.
//!
//! A [`Differ`] turns a before/after pair into a [`Patch`] and applies a
//! patch back. The interface is generic; [`by_value`] is the equality
//! differ (changed or not). Concrete structural differs (records, chunks)
//! belong to the collections wave; this module carries the contract plus
//! the value differ the ported rows need.

/// A patch between two values of `T`.
#[derive(Clone, Debug, PartialEq)]
pub enum Patch<T> {
    /// No change.
    Same,
    /// Replace with this value.
    Replace(T),
}

/// Bidirectional diff (`Differ<T, Patch>`).
pub struct Differ<T, P> {
    diff: Box<dyn Fn(&T, &T) -> P>,
    patch: Box<dyn Fn(&P, &T) -> T>,
}

impl<T: 'static, P: 'static> Differ<T, P> {
    /// Build from diff and patch functions.
    pub fn make(
        diff: impl Fn(&T, &T) -> P + 'static,
        patch: impl Fn(&P, &T) -> T + 'static,
    ) -> Self {
        Self {
            diff: Box::new(diff),
            patch: Box::new(patch),
        }
    }

    /// Compute the patch from old to new.
    pub fn diff(&self, old: &T, new: &T) -> P {
        (self.diff)(old, new)
    }

    /// Apply a patch to the old value.
    pub fn patch(&self, patch: &P, old: &T) -> T {
        (self.patch)(patch, old)
    }

    /// Diff then patch roundtrips (`patch(diff(a, b), a) == b`).
    pub fn roundtrip(&self, old: &T, new: &T) -> T
    where
        T: Clone,
    {
        let p = self.diff(old, new);
        self.patch(&p, old)
    }
}

/// Equality differ: `Same` when equal, `Replace` otherwise.
pub fn by_value<T: Clone + PartialEq + 'static>() -> Differ<T, Patch<T>> {
    Differ::make(
        |old: &T, new: &T| {
            if old == new {
                Patch::Same
            } else {
                Patch::Replace(new.clone())
            }
        },
        |patch, old| match patch {
            Patch::Same => old.clone(),
            Patch::Replace(v) => v.clone(),
        },
    )
}
