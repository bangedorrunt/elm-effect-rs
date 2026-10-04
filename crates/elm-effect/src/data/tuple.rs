//! `Tuple.ts` port: fixed small arities are native Rust tuples.
//! Pairs and triples need no wrapper: `(A, B)` is the shape. What's here:
//! the tag's named projections and extensions. Residuals: variadic appends
//! beyond triples (nest instead).

use super::combiner::Combiner;

/// First of a pair (`getFirst`).
pub fn get_first<A: Clone, B>(pair: &(A, B)) -> A {
    pair.0.clone()
}

/// Second of a pair (`getSecond`).
pub fn get_second<A, B: Clone>(pair: &(A, B)) -> B {
    pair.1.clone()
}

/// Swap a pair (`swap`).
pub fn swap<A, B>(pair: (A, B)) -> (B, A) {
    (pair.1, pair.0)
}

/// Extend a pair to a triple (`appendElement`).
pub fn append_element<A, B, C>(pair: (A, B), third: C) -> (A, B, C) {
    (pair.0, pair.1, third)
}

/// Extend a pair on the left (`prependElement`).
pub fn prepend_element<A, B, C>(first: C, pair: (A, B)) -> (C, A, B) {
    (first, pair.0, pair.1)
}

/// Map the first (`mapFirst`).
pub fn map_first<A, B, C>(pair: (A, B), f: impl FnOnce(A) -> C) -> (C, B) {
    (f(pair.0), pair.1)
}

/// Map the second (`mapSecond`).
pub fn map_second<A, B, C>(pair: (A, B), f: impl FnOnce(B) -> C) -> (A, C) {
    (pair.0, f(pair.1))
}

/// First of a triple (`getFirst` on triples reads `.0` natively; named here
/// for the row).
pub fn triple_first<A: Clone, B, C>(triple: &(A, B, C)) -> A {
    triple.0.clone()
}

/// Construct a pair (`make` shape for pairs).
pub fn make_pair<A, B>(first: A, second: B) -> (A, B) {
    (first, second)
}

/// Construct a triple (`make` shape for triples).
pub fn make_triple<A, B, C>(first: A, second: B, third: C) -> (A, B, C) {
    (first, second, third)
}

/// Get by runtime index on a homogeneous slice (`get` index shape).
pub fn get_at<T: Clone>(values: &[T], index: usize) -> Option<T> {
    values.get(index).cloned()
}

/// Keep only the listed positions (`pick` index shape).
pub fn pick_indices<T: Clone>(values: &[T], indices: &[usize]) -> Vec<T> {
    indices
        .iter()
        .filter_map(|i| values.get(*i).cloned())
        .collect()
}

/// Drop the listed positions (`omit` index shape).
pub fn omit_indices<T: Clone>(values: &[T], indices: &[usize]) -> Vec<T> {
    values
        .iter()
        .enumerate()
        .filter(|(i, _)| !indices.contains(i))
        .map(|(_, v)| v.clone())
        .collect()
}

/// Update one position (`evolve` index shape); out-of-bounds is a no-op.
pub fn evolve_at<T: Clone>(values: &[T], index: usize, f: impl FnOnce(T) -> T) -> Vec<T> {
    let mut out = values.to_vec();
    if let Some(slot) = out.get_mut(index) {
        let next = f(slot.clone());
        *slot = next;
    }
    out
}

/// Pairwise equality, positions independently (`makeEquivalence` shape).
pub fn pairs_equal<A: PartialEq, B: PartialEq>(a: &(A, B), b: &(A, B)) -> bool {
    a.0 == b.0 && a.1 == b.1
}

/// Lexicographic order (`makeOrder` shape).
pub fn cmp_pairs<A: Ord, B: Ord>(a: &(A, B), b: &(A, B)) -> std::cmp::Ordering {
    a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1))
}
pub fn make_combiner<A: 'static, B: 'static>(
    first: Combiner<A>,
    second: Combiner<B>,
) -> Combiner<(A, B)> {
    Combiner::make(move |(a1, b1): &(A, B), (a2, b2): &(A, B)| {
        (first.combine(a1, a2), second.combine(b1, b2))
    })
}
