//! `Option.ts` port over Rust's [`Option`]: only what `std` lacks.
//!
//! Mapping, chaining, filtering, and unwrapping are `std` (`map`, `and_then`,
//! `flatten`, `filter`, `filter_map`, `or_else`, `unwrap_or_else`,
//! `contains`, `is_some_and`, iterators). What's here: the tag's named
//! extras. Out of scope on purpose (owner beads named): do-notation
//! (`Do`/`bind`/`bindTo`/`let`), `makeEquivalence`/`makeOrder` (.17),
//! `makeReducer`/`makeReducerFailFast` (.19), `partitionMap`/`productMany`/
//! `reduceCompact`/`orElseResult`, `lift2` (.17), hash/structural equality
//! (.17), `gen` (needs the generator protocol).

/// Replace the value, keeping `None` (`as`).
pub fn as_<T, U>(value: Option<T>, replacement: U) -> Option<U> {
    value.map(|_| replacement)
}

/// Tap a value for observation, unchanged (`tap`; alias of `inspect`).
pub fn tap<T>(value: Option<T>, f: impl FnOnce(&T)) -> Option<T> {
    value.inspect(f)
}

/// First `Some` in order, or `None`.
pub fn first_some_of<T, I: IntoIterator<Item = Option<T>>>(iterable: I) -> Option<T> {
    iterable.into_iter().find(|o| o.is_some()).flatten()
}

/// First element of an iterable, or `None` (`fromIterable`).
pub fn from_iterable<T>(iterable: impl IntoIterator<Item = T>) -> Option<T> {
    iterable.into_iter().next()
}

/// Collapse one nullish layer (`fromNullishOr` on nested options).
pub fn from_nullish<T>(value: Option<Option<T>>) -> Option<T> {
    value.flatten()
}

/// Map then collapse one nullish layer (`flatMapNullishOr`).
pub fn flat_map_nullish<T, U>(
    value: Option<T>,
    f: impl FnOnce(T) -> Option<Option<U>>,
) -> Option<U> {
    value.and_then(|a| f(a).flatten())
}

/// Lift a nullish-returning function as-is: the type already says `Option`
/// (`liftNullishOr`).
pub fn lift_nullish_or<T, U, F: Fn(T) -> Option<U>>(f: F) -> F {
    f
}

/// Unwrap or panic with the tag's `getOrThrow` message.
pub fn get_or_throw<T>(value: Option<T>) -> T {
    value.expect("getOrThrow called on a None")
}

/// Unwrap or compute the fallback error.
pub fn get_or_throw_with<T, E>(value: Option<T>, on_none: impl FnOnce() -> E) -> Result<T, E> {
    value.ok_or_else(on_none)
}

/// `None` for `None`, `Some(())` for `Some` (`asVoid`).
pub fn as_void<T>(value: Option<T>) -> Option<()> {
    value.map(|_| ())
}

/// Fold both cases (`match`).
pub fn match_<T, U>(
    value: Option<T>,
    on_none: impl FnOnce() -> U,
    on_some: impl FnOnce(T) -> U,
) -> U {
    match value {
        None => on_none(),
        Some(v) => on_some(v),
    }
}

/// Recover with another option (`orElse`), or a default `Some` (`orElseSome`).
pub fn or_else_some<T>(value: Option<T>, default: T) -> Option<T> {
    value.or(Some(default))
}

/// Pair two present values (`product`, `tuple` shapes).
pub fn product<T, U>(a: Option<T>, b: Option<U>) -> Option<(T, U)> {
    zip_with(a, b, |a, b| (a, b))
}

/// Pair two present values (`tuple` shape).
pub fn tuple<T, U>(a: Option<T>, b: Option<U>) -> Option<(T, U)> {
    product(a, b)
}

/// Pair two options, short-circuiting on the first `None` (`zipWith`).
pub fn zip_with<T, U, V>(a: Option<T>, b: Option<U>, f: impl FnOnce(T, U) -> V) -> Option<V> {
    match (a, b) {
        (Some(a), Some(b)) => Some(f(a, b)),
        _ => None,
    }
}

/// Collect a list, short-circuiting on the first `None` (`all`).
pub fn all<T>(values: Vec<Option<T>>) -> Option<Vec<T>> {
    values.into_iter().collect()
}

/// Lift a predicate: `Some` stays iff it passes (`liftPredicate`).
pub fn lift_predicate<T>(predicate: impl Fn(&T) -> bool) -> impl Fn(T) -> Option<T> {
    move |value| if predicate(&value) { Some(value) } else { None }
}

/// Lift a throwing closure: panics become `None` (`liftThrowable`).
pub fn lift_throwable<T>(f: impl FnOnce() -> T) -> Option<T> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).ok()
}

/// Build a refinement from an option-returning function (`toRefinement`).
pub fn to_refinement<A, B>(f: impl Fn(&A) -> Option<B>) -> impl Fn(&A) -> bool {
    move |a| f(a).is_some()
}

/// The success channel of a `Result` (`getSuccess`).
pub fn get_success<T, E>(value: Result<T, E>) -> Option<T> {
    value.ok()
}

/// The failure channel of a `Result` (`getFailure`).
pub fn get_failure<T, E>(value: Result<T, E>) -> Option<E> {
    value.err()
}

/// `null` encoding for `None` (`getOrNull`); `Some` unwraps.
pub fn to_json<T: std::fmt::Display>(value: &Option<T>) -> String {
    match value {
        None => "null".to_string(),
        Some(v) => v.to_string(),
    }
}

// --- Wave-6 residuals (deferred from .16; Equal/Reducer deps landed) ---

use super::combiner::{Combiner, Reducer};
use super::foundation::Equal;

/// Chain, short-circuiting on `None` (`bind`, `flatMap` shape).
pub fn bind<T, U>(value: Option<T>, f: impl FnOnce(T) -> Option<U>) -> Option<U> {
    value.and_then(f)
}

/// Empty do-block (`Do`): the `Some` unit workshops below build on.
pub fn do_() -> Option<()> {
    Some(())
}

/// Run a do-block: `?` propagates `None` (`gen` shape; closures are the
/// notation, the operator is the short-circuit).
pub fn gen_block<T>(block: impl FnOnce() -> Option<T>) -> Option<T> {
    block()
}

/// Attach a bound value (`bindTo` tuple shape).
pub fn bind_to<T, U>(value: Option<T>, f: impl FnOnce(&T) -> U) -> Option<(T, U)> {
    match value {
        None => None,
        Some(v) => {
            let u = f(&v);
            Some((v, u))
        }
    }
}

/// Attach a computed value (`let` tuple shape).
pub fn let_<T, U>(value: Option<T>, f: impl FnOnce(&T) -> U) -> Option<(T, U)> {
    bind_to(value, f)
}

/// Keep `Some` only when `f` holds (`filter`; keeps do-records whose guard
/// passes).
pub fn filter<T>(value: Option<T>, f: impl FnOnce(&T) -> bool) -> Option<T> {
    match value {
        Some(v) if f(&v) => Some(v),
        _ => None,
    }
}

/// `true` when present and structurally equal (`containsWith`).
pub fn contains_with<T: Equal>(value: &Option<T>, item: &T) -> bool {
    match value {
        Some(v) => v.equal(item),
        None => false,
    }
}

/// Lift a two-argument function (`lift2`).
pub fn lift2<T, U, V>(f: impl FnOnce(T, U) -> V, a: Option<T>, b: Option<U>) -> Option<V> {
    match (a, b) {
        (Some(a), Some(b)) => Some(f(a, b)),
        _ => None,
    }
}

/// Fall back to a `Result` (`orElseEither` shape: `None` runs `that`).
pub fn or_else_result<T, E>(value: Option<T>, that: impl FnOnce() -> Result<T, E>) -> Result<T, E> {
    match value {
        Some(v) => Ok(v),
        None => that(),
    }
}

/// Split one option into left/right channels (`partitionMap` over `Result`:
/// `Ok` goes right, `Err` goes left, `None` goes nowhere).
pub fn partition_map<T, B, C>(
    value: Option<T>,
    f: impl FnOnce(T) -> Result<B, C>,
) -> (Option<C>, Option<B>) {
    match value {
        None => (None, None),
        Some(v) => match f(v) {
            Ok(b) => (None, Some(b)),
            Err(c) => (Some(c), None),
        },
    }
}

/// Collect many, short-circuiting on the first `None` (`productMany`).
pub fn product_many<T>(first: Option<T>, rest: Vec<Option<T>>) -> Option<Vec<T>> {
    let mut out = Vec::with_capacity(rest.len() + 1);
    out.push(first?);
    for item in rest {
        out.push(item?);
    }
    Some(out)
}

/// Fold present values, skipping `None` (`reduceCompact`).
pub fn reduce_compact<T, B>(value: Option<T>, zero: B, f: impl FnOnce(B, T) -> B) -> B {
    match value {
        Some(v) => f(zero, v),
        None => zero,
    }
}

/// Pair two present values as a record tuple (`struct` 2-shape).
pub fn struct2<T, U>(a: Option<T>, b: Option<U>) -> Option<(T, U)> {
    product(a, b)
}

/// Triple record tuple (`struct` 3-shape).
pub fn struct3<T, U, V>(a: Option<T>, b: Option<U>, c: Option<V>) -> Option<(T, U, V)> {
    match (a, b, c) {
        (Some(a), Some(b), Some(c)) => Some((a, b, c)),
        _ => None,
    }
}

/// Structural equality, `NaN`-aware via [`Equal`] (`getEquivalence` shape).
pub fn equal_with<T: Equal>(a: &Option<T>, b: &Option<T>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(x), Some(y)) => x.equal(y),
        _ => false,
    }
}

/// `None`-below-`Some` ordering (`getOrder` shape; `None` sorts least).
pub fn order_with<T: Ord>(a: &Option<T>, b: &Option<T>) -> std::cmp::Ordering {
    a.cmp(b)
}

/// Reducer that keeps the first `Some` (`makeReducer`): `None` is the
/// identity absorbed on either side; two `Some`s combine inside.
pub fn make_reducer<A: Clone + 'static>(combiner: Combiner<A>) -> Reducer<Option<A>> {
    Reducer {
        zero: None,
        combiner: Combiner::make(move |l: &Option<A>, r: &Option<A>| match (l, r) {
            (None, _) => r.clone(),
            (_, None) => l.clone(),
            (Some(a), Some(b)) => Some(combiner.combine(a, b)),
        }),
    }
}

/// Fail-fast reducer (`makeReducerFailFast`): any `None` annihilates.
pub fn make_reducer_fail_fast<A: Clone + 'static>(reducer: Reducer<A>) -> Reducer<Option<A>> {
    Reducer {
        zero: None,
        combiner: Combiner::make(move |l: &Option<A>, r: &Option<A>| match (l, r) {
            (Some(a), Some(b)) => Some(reducer.combiner.combine(a, b)),
            _ => None,
        }),
    }
}

/// The value itself (`getOrNull`/`getOrUndefined` collapse: Rust has no
/// null/undefined split, so both directions are the identity).
pub fn get_or_null<T>(value: Option<T>) -> Option<T> {
    value
}

/// Alias of [`get_or_null`]: null and undefined collapse in Rust.
pub fn get_or_undefined<T>(value: Option<T>) -> Option<T> {
    value
}
