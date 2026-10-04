//! `UndefinedOr.ts` port: TypeScript's `T | undefined` is Rust's [`Option`].
//!
//! The substrate needs no wrapper: every behavior below holds of [`Option`]
//! directly. What this module adds is the tag's throwing edge
//! (`getOrThrow` panics with the default missing error) and the throwable
//! lift.

/// TypeScript `T | undefined`.
pub type UndefinedOr<T> = Option<T>;

/// The default missing error, matching the tag's `getOrThrow` message shape.
pub const MISSING_ERROR: &str = "UndefinedOr is undefined";

/// Unwrap a defined value; panic with [`MISSING_ERROR`] on `None`.
/// Maps the tag's throw to a panic (documented at `option::get_or_throw`).
pub fn get_or_throw<T>(value: UndefinedOr<T>) -> T {
    value.expect(MISSING_ERROR)
}

/// Unwrap, or compute the fallback error value.
pub fn get_or_throw_with<T, E>(value: UndefinedOr<T>, missing: impl FnOnce() -> E) -> Result<T, E> {
    value.ok_or_else(missing)
}

/// Lift a throwing closure into `UndefinedOr`: panics become `None`.
pub fn lift_throwable<T>(f: impl FnOnce() -> T) -> UndefinedOr<T> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).ok()
}

// --- Wave-6 residuals (deferred from .16; Reducer dep landed) ---

use super::combiner::{Combiner, Reducer};
use super::option;

/// Fold both cases (`match`).
pub fn match_<T, U>(
    value: UndefinedOr<T>,
    on_undefined: impl FnOnce() -> U,
    on_defined: impl FnOnce(T) -> U,
) -> U {
    option::match_(value, on_undefined, on_defined)
}

/// Reducer that keeps the first defined value (`makeReducer`).
pub fn make_reducer<A: Clone + 'static>(combiner: Combiner<A>) -> Reducer<UndefinedOr<A>> {
    option::make_reducer(combiner)
}

/// Fail-fast reducer (`makeReducerFailFast`).
pub fn make_reducer_fail_fast<A: Clone + 'static>(reducer: Reducer<A>) -> Reducer<UndefinedOr<A>> {
    option::make_reducer_fail_fast(reducer)
}
