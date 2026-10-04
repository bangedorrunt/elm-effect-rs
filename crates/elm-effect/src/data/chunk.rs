//! `Chunk.ts` port: a persistent-style sequence. Backed by `Vec` (structural
//! sharing is an implementation detail; observable behavior is the spec).
//!
//! Residuals: `unsafeGet`/`unsafeFromArray` (no unchecked indexing in safe
//! Rust), cross-module makers (`.17`/`.19`).

/// An immutable sequence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chunk<T>(Vec<T>);

impl<T> Chunk<T> {
    /// Empty chunk.
    pub fn empty() -> Self {
        Self(Vec::new())
    }

    /// One element (`of`, `make` single).
    pub fn of(value: T) -> Self {
        Self(vec![value])
    }

    /// From any iterable (`fromIterable`, `fromArray`).
    pub fn from_iter<I: IntoIterator<Item = T>>(iterable: I) -> Self {
        Self(iterable.into_iter().collect())
    }

    /// Length (`size`).
    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// First element (`head`, `unsafeHead` without the partiality).
    pub fn head(&self) -> Option<&T> {
        self.0.first()
    }

    /// Last element (`last`).
    pub fn last(&self) -> Option<&T> {
        self.0.last()
    }

    /// All but the first (`tail`).
    pub fn tail(&self) -> Self
    where
        T: Clone,
    {
        Self(self.0.get(1..).unwrap_or(&[]).to_vec())
    }

    /// All but the last (`init`).
    pub fn init(&self) -> Self
    where
        T: Clone,
    {
        Self(
            self.0
                .get(..self.0.len().saturating_sub(1))
                .unwrap_or(&[])
                .to_vec(),
        )
    }

    /// Get by index (`get`).
    pub fn get(&self, index: usize) -> Option<&T> {
        self.0.get(index)
    }

    /// First `n` (`take`).
    pub fn take(&self, n: usize) -> Self
    where
        T: Clone,
    {
        Self(self.0.iter().take(n).cloned().collect())
    }

    /// Drop the first `n` (`drop`).
    pub fn drop(&self, n: usize) -> Self
    where
        T: Clone,
    {
        Self(self.0.iter().skip(n).cloned().collect())
    }

    /// Append one (`append`, `appendAll` for many).
    pub fn append(mut self, value: T) -> Self {
        self.0.push(value);
        self
    }

    /// Append many.
    pub fn append_all(mut self, values: impl IntoIterator<Item = T>) -> Self {
        self.0.extend(values);
        self
    }

    /// Prepend one (`prepend`, `prependAll`).
    pub fn prepend(self, value: T) -> Self {
        let mut out = Vec::with_capacity(self.0.len() + 1);
        out.push(value);
        out.extend(self.0);
        Self(out)
    }

    /// Concatenate (`concat`).
    pub fn concat(self, other: Self) -> Self {
        self.append_all(other.0)
    }

    /// Map (`map`).
    pub fn map<U>(self, mut f: impl FnMut(T) -> U) -> Chunk<U> {
        Chunk(self.0.into_iter().map(|a| f(a)).collect())
    }

    /// Filter (`filter`).
    pub fn filter(mut self, mut predicate: impl FnMut(&T) -> bool) -> Self {
        self.0.retain(|a| predicate(a));
        self
    }

    /// Chain (`flatMap`).
    pub fn flat_map<U>(self, mut f: impl FnMut(T) -> Chunk<U>) -> Chunk<U> {
        Chunk(self.0.into_iter().flat_map(|a| f(a).0).collect())
    }

    /// Flatten one level (`flatten`).
    pub fn flatten(self) -> Chunk<T::Item>
    where
        T: IntoIterator,
        T::Item: Clone,
    {
        Chunk(self.0.into_iter().flatten().collect())
    }

    /// Reverse (`reverse`).
    pub fn reverse(mut self) -> Self {
        self.0.reverse();
        self
    }

    /// Split at `n` (`splitAt`).
    pub fn split_at(&self, n: usize) -> (Self, Self)
    where
        T: Clone,
    {
        let (a, b) = self.0.split_at(n.min(self.0.len()));
        (Self(a.to_vec()), Self(b.to_vec()))
    }

    /// Pairwise zip, stopping at the shorter (`zip`).
    pub fn zip<U>(self, other: Chunk<U>) -> Chunk<(T, U)> {
        Chunk(self.0.into_iter().zip(other.0).collect())
    }

    /// Collect successes, short-circuiting on `Err` (`collectAll` shape for
    /// `Result`).
    pub fn collect_results<E>(values: Vec<Result<T, E>>) -> Result<Self, E> {
        values.into_iter().collect::<Result<Vec<_>, _>>().map(Self)
    }

    /// As a slice for interop.
    pub fn as_slice(&self) -> &[T] {
        &self.0
    }

    /// Into the backing vector.
    pub fn into_vec(self) -> Vec<T> {
        self.0
    }
}

impl<T> Default for Chunk<T> {
    fn default() -> Self {
        Self::empty()
    }
}

impl<T> FromIterator<T> for Chunk<T> {
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        Self::from_iter(iter)
    }
}
