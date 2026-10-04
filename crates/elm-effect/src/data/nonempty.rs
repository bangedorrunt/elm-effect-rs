//! `NonEmptyIterable.ts` port: an iterable with a statically known head.
//!
//! Residuals: tuple-destructuring helpers (`unprepend` shapes beyond head).

/// A head plus a (possibly empty) tail: never empty by construction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NonEmpty<T> {
    /// Guaranteed present.
    pub head: T,
    /// The rest, possibly empty.
    pub tail: Vec<T>,
}

impl<T> NonEmpty<T> {
    /// Build from head and tail (`make`, `of` for head-only).
    pub fn make(head: T, tail: Vec<T>) -> Self {
        Self { head, tail }
    }

    /// A single element.
    pub fn of(head: T) -> Self {
        Self::make(head, Vec::new())
    }

    /// From a vector: `None` when empty (`fromIterable`).
    pub fn from_vec(mut values: Vec<T>) -> Option<Self> {
        if values.is_empty() {
            None
        } else {
            let head = values.remove(0);
            Some(Self::make(head, values))
        }
    }

    /// The head (`head`).
    pub fn head(&self) -> &T {
        &self.head
    }

    /// The tail (`tail`).
    pub fn tail(&self) -> &[T] {
        &self.tail
    }

    /// The last element (`last`).
    pub fn last(&self) -> &T {
        self.tail.last().unwrap_or(&self.head)
    }

    /// Element count, at least 1 (`size`).
    pub fn len(&self) -> usize {
        1 + self.tail.len()
    }

    pub fn is_empty(&self) -> bool {
        false
    }

    /// Map preserving non-emptiness (`map`).
    pub fn map<U>(self, mut f: impl FnMut(T) -> U) -> NonEmpty<U> {
        let head = f(self.head);
        let tail = self.tail.into_iter().map(|a| f(a)).collect();
        NonEmpty::make(head, tail)
    }

    /// Flatten one level (`flatMap` with non-empty results stays non-empty
    /// only if the head maps non-empty; general shape returns `Vec`).
    pub fn flat_map<U>(self, mut f: impl FnMut(T) -> Vec<U>) -> Vec<U> {
        let mut out = f(self.head);
        out.extend(self.tail.into_iter().flat_map(|a| f(a)));
        out
    }

    /// All elements head-first (`toArray`, iteration order).
    pub fn to_vec(self) -> Vec<T> {
        let mut out = Vec::with_capacity(1 + self.tail.len());
        out.push(self.head);
        out.extend(self.tail);
        out
    }

    /// Reverse, still non-empty (`reverse`).
    pub fn reverse(self) -> Self {
        let mut all = self.to_vec();
        all.reverse();
        Self::from_vec(all).expect("reverse of non-empty is non-empty")
    }
}

impl<T> IntoIterator for NonEmpty<T> {
    type Item = T;
    type IntoIter = std::vec::IntoIter<T>;

    fn into_iter(self) -> Self::IntoIter {
        self.to_vec().into_iter()
    }
}
