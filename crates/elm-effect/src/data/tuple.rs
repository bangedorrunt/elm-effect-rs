//! `Tuple.ts` port: fixed small arities are native Rust tuples.
//!
//! Pairs and triples need no wrapper: `(A, B)` is the shape. What's here:
//! the tag's named projections and extensions. Residuals: variadic appends
//! beyond triples (nest instead).

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
