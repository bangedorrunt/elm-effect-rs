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
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(v) => Some(v),
        Err(_) => None,
    }
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
