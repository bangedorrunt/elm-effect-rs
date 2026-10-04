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

    /// Element count (`size` alias).
    pub fn size(&self) -> usize {
        self.len()
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
    pub fn map<U>(self, f: impl FnMut(T) -> U) -> Chunk<U> {
        Chunk(self.0.into_iter().map(f).collect())
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

    /// As a slice for interop (kept for upcoming beads).
    #[allow(dead_code)]
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

impl<T> Chunk<T> {
    /// Replace by index, cloning the rest (`modify`).
    pub fn modify(&self, index: usize, f: impl FnOnce(T) -> T) -> Self
    where
        T: Clone,
    {
        let mut out = self.0.clone();
        if let Some(slot) = out.get_mut(index) {
            let next = f(slot.clone());
            *slot = next;
        }
        Self(out)
    }

    /// Replace by index (`replace`).
    pub fn replace(&self, index: usize, value: T) -> Self
    where
        T: Clone,
    {
        self.modify(index, |_| value)
    }

    /// Remove by index (`remove`).
    pub fn remove(&self, index: usize) -> Self
    where
        T: Clone,
    {
        let mut out = self.0.clone();
        if index < out.len() {
            out.remove(index);
        }
        Self(out)
    }

    /// Take while the predicate holds (`takeWhile`).
    pub fn take_while(&self, mut predicate: impl FnMut(&T) -> bool) -> Self
    where
        T: Clone,
    {
        Self(
            self.0
                .iter()
                .take_while(|a| predicate(a))
                .cloned()
                .collect(),
        )
    }

    /// Drop while the predicate holds.
    pub fn drop_while(&self, mut predicate: impl FnMut(&T) -> bool) -> Self
    where
        T: Clone,
    {
        let mut skipped = self.0.iter().skip_while(|a| predicate(a));
        Self(std::iter::from_fn(move || skipped.next().cloned()).collect())
    }

    /// Split on the first predicate match (`splitWhere`).
    pub fn split_where(&self, predicate: impl FnMut(&T) -> bool) -> (Self, Self)
    where
        T: Clone,
    {
        match self.0.iter().position(predicate) {
            Some(at) => (Self(self.0[..at].to_vec()), Self(self.0[at..].to_vec())),
            None => (self.clone(), Self::empty()),
        }
    }

    /// Stateful map (`mapAccum`).
    pub fn map_accum<S, U>(&self, init: S, mut f: impl FnMut(S, &T) -> (S, U)) -> (S, Chunk<U>)
    where
        T: Clone,
    {
        let mut state = init;
        let mut out = Vec::with_capacity(self.0.len());
        for v in &self.0 {
            let (next, u) = f(state, v);
            state = next;
            out.push(u);
        }
        (state, Chunk(out))
    }

    /// Split by predicate (`partition`).
    pub fn partition(&self, mut predicate: impl FnMut(&T) -> bool) -> (Self, Self)
    where
        T: Clone,
    {
        let mut yes = Vec::new();
        let mut no = Vec::new();
        for v in &self.0 {
            if predicate(v) {
                yes.push(v.clone());
            } else {
                no.push(v.clone());
            }
        }
        (Self(yes), Self(no))
    }

    /// Split `Ok`/`Err` (`separate`).
    pub fn separate<T2, E>(chunk: Chunk<Result<T2, E>>) -> (Chunk<T2>, Chunk<E>) {
        let mut oks = Vec::new();
        let mut errs = Vec::new();
        for r in chunk.0 {
            match r {
                Ok(v) => oks.push(v),
                Err(e) => errs.push(e),
            }
        }
        (Chunk(oks), Chunk(errs))
    }

    /// Last match (`findLast`).
    pub fn find_last(&self, mut predicate: impl FnMut(&T) -> bool) -> Option<&T> {
        self.0.iter().rev().find(|a| predicate(a))
    }

    /// Map-then-collect, dropping `None`s (`filterMap`).
    pub fn filter_map<U>(&self, f: impl FnMut(&T) -> Option<U>) -> Chunk<U>
    where
        T: Clone,
    {
        Chunk(self.0.iter().filter_map(f).collect())
    }

    /// Drop `None`s, unwrapping (`compact`).
    pub fn compact<U>(chunk: &Chunk<Option<U>>) -> Chunk<U>
    where
        U: Clone,
    {
        Chunk(chunk.0.iter().filter_map(|o| o.clone()).collect())
    }

    /// Deduplicate adjacent equals (`dedupeAdjacent`, `dedupe`).
    pub fn dedupe(&self) -> Self
    where
        T: Clone + PartialEq,
    {
        let mut out = self.0.clone();
        out.dedup();
        Self(out)
    }

    /// Unzip pairs (`unzip`).
    pub fn unzip<A, B>(chunk: Chunk<(A, B)>) -> (Chunk<A>, Chunk<B>) {
        let (a, b): (Vec<_>, Vec<_>) = chunk.0.into_iter().unzip();
        (Chunk(a), Chunk(b))
    }

    /// Build from a generator (`makeBy`).
    pub fn make_by(n: usize, f: impl FnMut(usize) -> T) -> Self {
        Self((0..n).map(f).collect())
    }

    /// Integer range (`range`).
    pub fn range(start: i32, end: i32) -> Chunk<i32> {
        Chunk((start..end).collect())
    }

    /// True when any element passes (`some`).
    pub fn some(&self, predicate: impl FnMut(&T) -> bool) -> bool {
        self.0.iter().any(predicate)
    }

    /// Run a side effect per element (`forEach`).
    pub fn for_each(&self, f: impl FnMut(&T)) {
        self.0.iter().for_each(f);
    }

    /// Sort by comparator (`sortWith`).
    pub fn sort_by<F>(&self, mut compare: F) -> Self
    where
        T: Clone,
        F: FnMut(&T, &T) -> std::cmp::Ordering,
    {
        let mut out = self.0.clone();
        out.sort_by(|a, b| compare(a, b));
        Self(out)
    }

    /// Chunks of `n` (`chunksOf`).
    pub fn chunks_of(&self, n: usize) -> Chunk<Chunk<T>>
    where
        T: Clone,
    {
        assert!(n > 0, "chunksOf requires n > 0");
        Chunk(self.0.chunks(n).map(|c| Self(c.to_vec())).collect())
    }

    /// Get or panic with the tag message (`getOrThrow` shape; panics).
    pub fn get_or_throw(&self, index: usize) -> &T {
        self.0.get(index).expect("getOrThrow called out of bounds")
    }

    /// Set union preserving first-seen order (`union`).
    pub fn union(&self, other: &Self) -> Self
    where
        T: Clone + PartialEq,
    {
        let mut out = self.0.clone();
        for v in &other.0 {
            if !out.contains(v) {
                out.push(v.clone());
            }
        }
        Self(out)
    }

    /// Intersection preserving order (`intersection`).
    pub fn intersection(&self, other: &Self) -> Self
    where
        T: Clone + PartialEq,
    {
        Self(
            self.0
                .iter()
                .filter(|v| other.0.contains(v))
                .cloned()
                .collect(),
        )
    }

    /// Difference preserving order (`difference`).
    pub fn difference(&self, other: &Self) -> Self
    where
        T: Clone + PartialEq,
    {
        Self(
            self.0
                .iter()
                .filter(|v| !other.0.contains(v))
                .cloned()
                .collect(),
        )
    }
}
