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

/// One element (`of`).
pub fn of<T>(value: T) -> Vec<T> {
    vec![value]
}

/// Prepend one (`prepend`).
pub fn prepend<T>(value: T, iterable: impl IntoIterator<Item = T>) -> Vec<T> {
    let mut out = vec![value];
    out.extend(iterable);
    out
}

/// Append one (`append`).
pub fn append<T>(iterable: impl IntoIterator<Item = T>, value: T) -> Vec<T> {
    let mut out: Vec<T> = iterable.into_iter().collect();
    out.push(value);
    out
}

/// Prefix accumulations (`scan`).
pub fn scan<T, U: Clone>(
    iterable: impl IntoIterator<Item = T>,
    init: U,
    mut f: impl FnMut(U, T) -> U,
) -> Vec<U> {
    let mut acc = init;
    let mut out = Vec::new();
    for v in iterable {
        acc = f(acc, v);
        out.push(acc.clone());
    }
    out
}

/// First `n` (`take`).
pub fn take<T>(iterable: impl IntoIterator<Item = T>, n: usize) -> Vec<T> {
    iterable.into_iter().take(n).collect()
}

/// Drop the first `n` (`drop`).
pub fn drop_<T>(iterable: impl IntoIterator<Item = T>, n: usize) -> Vec<T> {
    iterable.into_iter().skip(n).collect()
}

/// Take while the predicate holds (`takeWhile`).
pub fn take_while<T>(
    iterable: impl IntoIterator<Item = T>,
    mut predicate: impl FnMut(&T) -> bool,
) -> Vec<T> {
    iterable.into_iter().take_while(|a| predicate(a)).collect()
}

/// Pairwise combine (`zip`, `zipWith`).
pub fn zip_with<T, U, V>(
    a: impl IntoIterator<Item = T>,
    b: impl IntoIterator<Item = U>,
    mut f: impl FnMut(T, U) -> V,
) -> Vec<V> {
    a.into_iter()
        .zip(b.into_iter())
        .map(|(x, y)| f(x, y))
        .collect()
}

/// Interleave a separator (`intersperse`).
pub fn intersperse<T: Clone>(iterable: impl IntoIterator<Item = T>, separator: T) -> Vec<T> {
    let base: Vec<T> = iterable.into_iter().collect();
    let mut out = Vec::with_capacity(base.len() * 2);
    for (i, v) in base.into_iter().enumerate() {
        if i > 0 {
            out.push(separator.clone());
        }
        out.push(v);
    }
    out
}

/// Unfold from a seed (`unfold`).
pub fn unfold<S, T>(mut seed: S, mut f: impl FnMut(&mut S) -> Option<T>) -> Vec<T> {
    let mut out = Vec::new();
    while let Some(v) = f(&mut seed) {
        out.push(v);
    }
    out
}

/// Keep defined values (`getSomes`).
pub fn get_somes<T: Clone>(iterable: impl IntoIterator<Item = Option<T>>) -> Vec<T> {
    iterable.into_iter().flatten().collect()
}

/// Cartesian product (`cartesian`, `cartesianWith` with pair shape).
pub fn cartesian<T: Clone, U: Clone>(
    a: impl IntoIterator<Item = T>,
    b: impl IntoIterator<Item = U>,
) -> Vec<(T, U)> {
    let b: Vec<U> = b.into_iter().collect();
    a.into_iter()
        .flat_map(|x| b.iter().map(move |y| (x.clone(), y.clone())))
        .collect()
}

/// Group consecutive equals (`groupWith`, `groupBy`, `group`).
pub fn group_by<T: Clone + PartialEq>(iterable: impl IntoIterator<Item = T>) -> Vec<Vec<T>> {
    let mut out: Vec<Vec<T>> = Vec::new();
    for v in iterable {
        match out.last_mut() {
            Some(last) if last.first() == Some(&v) => last.push(v),
            _ => out.push(vec![v]),
        }
    }
    out
}

/// Take-while plus map-filter in one pass (`filterMapWhile`).
pub fn filter_map_while<T, U>(
    iterable: impl IntoIterator<Item = T>,
    mut take: impl FnMut(&T) -> bool,
    mut f: impl FnMut(T) -> Option<U>,
) -> Vec<U> {
    let mut taking = true;
    iterable
        .into_iter()
        .take_while(|a| {
            taking = taking && take(a);
            taking
        })
        .filter_map(|a| f(a))
        .collect()
}

/// Map then collapse one nullish layer (`flatMapNullishOr`).
pub fn flat_map_nullish_or<T, U>(
    iterable: impl IntoIterator<Item = T>,
    mut f: impl FnMut(T) -> Option<Option<U>>,
) -> Vec<U> {
    iterable
        .into_iter()
        .filter_map(|a| f(a).flatten())
        .collect()
}

/// Chunks of `n` (`chunksOf`).
pub fn chunks_of<T: Clone>(iterable: impl IntoIterator<Item = T>, n: usize) -> Vec<Vec<T>> {
    assert!(n > 0, "chunksOf requires n > 0");
    let mut out = Vec::new();
    let mut cur = Vec::new();
    for v in iterable {
        cur.push(v);
        if cur.len() == n {
            out.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}
