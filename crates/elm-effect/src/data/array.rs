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

/// Take the last `n` (`takeRight`).
pub fn take_right<T: Clone>(values: &[T], n: usize) -> Vec<T> {
    let start = values.len().saturating_sub(n);
    values.get(start..).unwrap_or(&[]).to_vec()
}

/// Drop the last `n` (`dropRight`).
pub fn drop_right<T: Clone>(values: &[T], n: usize) -> Vec<T> {
    let end = values.len().saturating_sub(n);
    values.get(..end).unwrap_or(&[]).to_vec()
}

/// Split at index (`splitAt`).
pub fn split_at<T: Clone>(values: &[T], n: usize) -> (Vec<T>, Vec<T>) {
    let at = n.min(values.len());
    (values[..at].to_vec(), values[at..].to_vec())
}

/// Insert at index, clamping to the end (`insertAt`).
pub fn insert_at<T: Clone>(values: &[T], index: usize, value: T) -> Vec<T> {
    let mut out = values.to_vec();
    out.insert(index.min(out.len()), value);
    out
}

/// Update by index through a function (`modify`).
pub fn modify<T: Clone>(values: &[T], index: usize, f: impl FnOnce(T) -> T) -> Vec<T> {
    let mut out = values.to_vec();
    if index < out.len() {
        let next = f(out[index].clone());
        out[index] = next;
    }
    out
}

/// Interleave a separator (`intersperse`).
pub fn intersperse<T: Clone>(values: &[T], separator: T) -> Vec<T> {
    let mut out = Vec::with_capacity(values.len() * 2);
    for (i, v) in values.iter().enumerate() {
        if i > 0 {
            out.push(separator.clone());
        }
        out.push(v.clone());
    }
    out
}

/// Rotate left by `n` (`rotate`).
pub fn rotate<T: Clone>(values: &[T], n: usize) -> Vec<T> {
    if values.is_empty() {
        return Vec::new();
    }
    let at = n % values.len();
    let mut out = values[at..].to_vec();
    out.extend_from_slice(&values[..at]);
    out
}

/// Prefix accumulations (`scan`).
pub fn scan<T: Clone, U: Clone>(values: &[T], init: U, mut f: impl FnMut(U, &T) -> U) -> Vec<U> {
    let mut acc = init;
    let mut out = Vec::with_capacity(values.len());
    for v in values {
        acc = f(acc, v);
        out.push(acc.clone());
    }
    out
}

/// Suffix accumulations (`scanRight`).
pub fn scan_right<T: Clone, U: Clone>(
    values: &[T],
    init: U,
    mut f: impl FnMut(U, &T) -> U,
) -> Vec<U> {
    let mut acc = init;
    let mut out = Vec::with_capacity(values.len());
    for v in values.iter().rev() {
        acc = f(acc, v);
        out.push(acc.clone());
    }
    out.reverse();
    out
}

/// Chop into pieces via a splitter (`chop`).
pub fn chop<T: Clone, U>(values: &[T], mut f: impl FnMut(&[T]) -> (U, Vec<T>)) -> Vec<U> {
    let mut rest = values.to_vec();
    let mut out = Vec::new();
    while !rest.is_empty() {
        let (piece, next) = f(&rest);
        out.push(piece);
        rest = next;
    }
    out
}

/// Pad to length (`pad`).
pub fn pad<T: Clone>(values: &[T], len: usize, pad: T) -> Vec<T> {
    let mut out = values.to_vec();
    while out.len() < len {
        out.push(pad.clone());
    }
    out
}

/// Allocate defaults (`allocate`).
pub fn allocate<T: Default + Clone>(n: usize) -> Vec<T> {
    vec![T::default(); n]
}

/// Unfold from a seed (`unfold`); `None` ends the stream.
pub fn unfold<S, T>(mut seed: S, mut f: impl FnMut(&mut S) -> Option<T>) -> Vec<T> {
    let mut out = Vec::new();
    while let Some(v) = f(&mut seed) {
        out.push(v);
    }
    out
}

/// Set union preserving first-seen order (`union`).
pub fn union<T: Clone + PartialEq>(a: &[T], b: &[T]) -> Vec<T> {
    let mut out = a.to_vec();
    for v in b {
        if !out.contains(v) {
            out.push(v.clone());
        }
    }
    out
}

/// Set intersection preserving left order (`intersection`).
pub fn intersection<T: Clone + PartialEq>(a: &[T], b: &[T]) -> Vec<T> {
    a.iter().filter(|v| b.contains(v)).cloned().collect()
}

/// Set difference preserving left order (`difference`).
pub fn difference<T: Clone + PartialEq>(a: &[T], b: &[T]) -> Vec<T> {
    a.iter().filter(|v| !b.contains(v)).cloned().collect()
}

/// Sliding windows of `n` (`window`).
pub fn window<T: Clone>(values: &[T], n: usize) -> Vec<Vec<T>> {
    if n == 0 || values.len() < n {
        return Vec::new();
    }
    (0..=values.len() - n)
        .map(|i| values[i..i + n].to_vec())
        .collect()
}

/// Stateful map (`mapAccum`).
pub fn map_accum<S, T, U>(
    values: &[T],
    init: S,
    mut f: impl FnMut(S, &T) -> (S, U),
) -> (S, Vec<U>) {
    let mut state = init;
    let mut out = Vec::with_capacity(values.len());
    for v in values {
        let (next, u) = f(state, v);
        state = next;
        out.push(u);
    }
    (state, out)
}

/// Count matches (`countBy`).
pub fn count_by<T>(values: &[T], mut predicate: impl FnMut(&T) -> bool) -> usize {
    values.iter().filter(|v| predicate(v)).count()
}
