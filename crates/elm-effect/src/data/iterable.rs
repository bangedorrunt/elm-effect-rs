//! `Iterable.ts` port: pull-based helpers over `IntoIterator`.
//!
//! Most behavior is `std` iterators; these names mirror the tag. Residuals:
//! infinite `cycle` (only bounded `cycle_n`), `concat` of heterogeneous
//! shapes, `reduceRight`.

/// First element (`head`).
pub fn first<T>(iterable: impl IntoIterator<Item = T>) -> Option<T> {
    iterable.into_iter().next()
}

/// Last element (`last`).
pub fn last<T>(iterable: impl IntoIterator<Item = T>) -> Option<T> {
    iterable.into_iter().last()
}

/// Element count (`size`, `countBy` with a predicate folded in).
pub fn count<T>(iterable: impl IntoIterator<Item = T>) -> usize {
    iterable.into_iter().count()
}

/// True when no elements (`isEmpty`).
pub fn is_empty<T>(iterable: impl IntoIterator<Item = T>) -> bool {
    iterable.into_iter().next().is_none()
}

/// First match (`findFirst`, `findLast` scans fully).
pub fn find_first<T>(
    iterable: impl IntoIterator<Item = T>,
    mut predicate: impl FnMut(&T) -> bool,
) -> Option<T> {
    iterable.into_iter().find(|a| predicate(a))
}

/// Last match (`findLast`).
pub fn find_last<T>(
    iterable: impl IntoIterator<Item = T>,
    predicate: impl Fn(&T) -> bool,
) -> Option<T> {
    iterable.into_iter().filter(|a| predicate(a)).last()
}

/// Contains by equality (`contains`, `containsWith` with `==`).
pub fn contains<T: PartialEq>(iterable: impl IntoIterator<Item = T>, value: &T) -> bool {
    iterable.into_iter().any(|a| a == *value)
}

/// Fold left (`reduce`).
pub fn reduce<T, U>(iterable: impl IntoIterator<Item = T>, init: U, f: impl FnMut(U, T) -> U) -> U {
    iterable.into_iter().fold(init, f)
}

/// Repeat `n` times, bounded (`cycle` made finite).
pub fn cycle_n<T: Clone>(iterable: impl IntoIterator<Item = T>, n: usize) -> Vec<T> {
    let base: Vec<T> = iterable.into_iter().collect();
    if base.is_empty() || n == 0 {
        return Vec::new();
    }
    base.iter().cycle().take(n * base.len()).cloned().collect()
}

/// Concatenate many (`concat`, `appendAll`).
pub fn concat_all<T>(iterables: impl IntoIterator<Item = Vec<T>>) -> Vec<T> {
    iterables.into_iter().flatten().collect()
}

/// Collect to a vector (`toArray`, `fromIterable`).
pub fn to_vec<T>(iterable: impl IntoIterator<Item = T>) -> Vec<T> {
    iterable.into_iter().collect()
}

/// Join strings (`join`, `mkString`).
pub fn join<'a>(iterable: impl IntoIterator<Item = &'a str>, separator: &str) -> String {
    iterable.into_iter().collect::<Vec<_>>().join(separator)
}

/// Minimum by key (`minBy`, `min` for `Ord`).
pub fn min<T: Ord>(iterable: impl IntoIterator<Item = T>) -> Option<T> {
    iterable.into_iter().min()
}

/// Maximum by key (`maxBy`, `max` for `Ord`).
pub fn max<T: Ord>(iterable: impl IntoIterator<Item = T>) -> Option<T> {
    iterable.into_iter().max()
}
