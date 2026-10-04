//! `Array.ts` port over `Vec`: only what `std` slices lack as named ops.
//!
//! `map`/`filter`/`fold`/`flatten`/`concat`/`reverse`/`sort`/`contains`/
//! iterators are `std`. Residuals (other beads or no Rust form): `Do`
//! notation, `makeEquivalence`/`makeOrder` (.17), `groupBy`, `shuffle`,
//! `cartesian`, zips beyond pairs.

use std::ops::Add;

/// First element, cloned (`head`).
pub fn head<T: Clone>(values: &[T]) -> Option<T> {
    values.first().cloned()
}

/// Last element, cloned (`last`).
pub fn last<T: Clone>(values: &[T]) -> Option<T> {
    values.last().cloned()
}

/// Everything but the first (`tail`).
pub fn tail<T: Clone>(values: &[T]) -> Vec<T> {
    values.get(1..).unwrap_or(&[]).to_vec()
}

/// Everything but the last (`init`).
pub fn init<T: Clone>(values: &[T]) -> Vec<T> {
    values
        .get(..values.len().saturating_sub(1))
        .unwrap_or(&[])
        .to_vec()
}

/// First `n` (`take`).
pub fn take<T: Clone>(values: &[T], n: usize) -> Vec<T> {
    values.iter().take(n).cloned().collect()
}

/// Drop the first `n` (`drop`).
pub fn drop_<T: Clone>(values: &[T], n: usize) -> Vec<T> {
    values.iter().skip(n).cloned().collect()
}

/// Take while the predicate holds (`takeWhile`).
pub fn take_while<T: Clone>(values: &[T], mut predicate: impl FnMut(&T) -> bool) -> Vec<T> {
    values
        .iter()
        .take_while(|a| predicate(a))
        .cloned()
        .collect()
}

/// Drop while the predicate holds (`dropWhile`).
pub fn drop_while<T: Clone>(values: &[T], mut predicate: impl FnMut(&T) -> bool) -> Vec<T> {
    let mut skipped = values.iter().skip_while(|a| predicate(a));
    std::iter::from_fn(move || skipped.next().cloned()).collect()
}

/// Build from a generator (`makeBy`).
pub fn make_by<T>(n: usize, mut f: impl FnMut(usize) -> T) -> Vec<T> {
    (0..n).map(|i| f(i)).collect()
}

/// Integer range `[start, end)` (`range`).
pub fn range(start: i32, end: i32) -> Vec<i32> {
    (start..end).collect()
}

/// Repeat a value (`replicate`).
pub fn replicate<T: Clone>(n: usize, value: T) -> Vec<T> {
    std::iter::repeat(value).take(n).collect()
}

/// Pairwise combine, stopping at the shorter (`zipWith`, `zip`).
pub fn zip_with<T, U, V>(a: &[T], b: &[U], mut f: impl FnMut(&T, &U) -> V) -> Vec<V> {
    a.iter().zip(b.iter()).map(|(x, y)| f(x, y)).collect()
}

/// Split into chunks of `n` (`chunksOf`).
pub fn chunks_of<T: Clone>(values: &[T], n: usize) -> Vec<Vec<T>> {
    assert!(n > 0, "chunksOf requires n > 0");
    values.chunks(n).map(|c| c.to_vec()).collect()
}

/// First index passing the predicate (`findFirstIndex` shape).
pub fn find_index<T>(values: &[T], mut predicate: impl FnMut(&T) -> bool) -> Option<usize> {
    values.iter().position(|a| predicate(a))
}

/// Get by index (`get`).
pub fn get<T: Clone>(values: &[T], index: usize) -> Option<T> {
    values.get(index).cloned()
}

/// Persistent set by index; out-of-bounds returns the input unchanged (`set`).
pub fn set<T: Clone>(values: &[T], index: usize, value: T) -> Vec<T> {
    let mut out = values.to_vec();
    if let Some(slot) = out.get_mut(index) {
        *slot = value;
    }
    out
}

/// Remove by index (`remove`); out-of-bounds is a no-op.
pub fn remove<T: Clone>(values: &[T], index: usize) -> Vec<T> {
    let mut out = values.to_vec();
    if index < out.len() {
        out.remove(index);
    }
    out
}

/// Sum with `+` (`sum`).
pub fn sum<T: Add<Output = T> + Default + Copy>(values: &[T]) -> T {
    values.iter().fold(T::default(), |acc, a| acc + *a)
}

/// Join strings (`join`, `mkString` with separator).
pub fn join(values: &[&str], separator: &str) -> String {
    values.join(separator)
}
