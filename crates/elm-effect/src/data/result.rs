//! `Result.ts` port over Rust's [`Result`]: only what `std` lacks.
//!
//! `map`/`map_err`/`and_then`/`or_else`/`unwrap_or_else`/`ok`/`err` are
//! `std`. What's here: the tag's named extras. Out of scope on purpose:
//! do-notation (`Do`/`bind`/`bindTo`/`let`), `gen`, `makeEquivalence` and the
//! `Equal` row (.17), `pipe` (no pipe protocol in Rust).

/// Wrap a success (`succeed`, `succeedSome`, `succeedNone` shapes live at the
/// call site: `Ok` / `Ok(Some(..))` / `Ok(None)`).
pub fn succeed<T>(value: T) -> Result<T, std::convert::Infallible> {
    Ok(value)
}

/// Wrap a failure (`fail`, `failVoid` shape is `Err(())`).
pub fn fail<E>(error: E) -> Result<std::convert::Infallible, E> {
    Err(error)
}

/// True for `Ok` (`isSuccess`); `isFailure` is `is_err`.
pub fn is_success<T, E>(value: &Result<T, E>) -> bool {
    value.is_ok()
}

/// Map both channels at once (`mapBoth`).
pub fn map_both<T, E, U, F>(
    value: Result<T, E>,
    on_success: impl FnOnce(T) -> U,
    on_failure: impl FnOnce(E) -> F,
) -> Result<U, F> {
    match value {
        Ok(v) => Ok(on_success(v)),
        Err(e) => Err(on_failure(e)),
    }
}

/// Swap the channels (`flip`).
pub fn flip<T, E>(value: Result<T, E>) -> Result<E, T> {
    match value {
        Ok(v) => Err(v),
        Err(e) => Ok(e),
    }
}

/// Collapse when both channels agree (`merge`).
pub fn merge<T>(value: Result<T, T>) -> T {
    match value {
        Ok(v) => v,
        Err(v) => v,
    }
}

/// Fold both cases (`match`).
pub fn match_<T, E, U>(
    value: Result<T, E>,
    on_failure: impl FnOnce(E) -> U,
    on_success: impl FnOnce(T) -> U,
) -> U {
    match value {
        Ok(v) => on_success(v),
        Err(e) => on_failure(e),
    }
}

/// Build from an option (`fromOption`, `fromNullishOr` on options).
pub fn from_option<T, E>(value: Option<T>, on_none: impl FnOnce() -> E) -> Result<T, E> {
    value.ok_or_else(on_none)
}

/// Move the option inside out (`transposeOption`).
pub fn transpose_option<T, E>(value: Result<Option<T>, E>) -> Option<Result<T, E>> {
    match value {
        Ok(None) => None,
        Ok(Some(v)) => Some(Ok(v)),
        Err(e) => Some(Err(e)),
    }
}

/// Keep successes passing a predicate (`filterOrFail`, `liftPredicate` shape).
pub fn filter_or_fail<T, E>(
    value: Result<T, E>,
    predicate: impl FnOnce(&T) -> bool,
    on_false: impl FnOnce(T) -> E,
) -> Result<T, E> {
    match value {
        Ok(v) if predicate(&v) => Ok(v),
        Ok(v) => Err(on_false(v)),
        Err(e) => Err(e),
    }
}

/// Observe a success, unchanged (`tap`; alias of `inspect`).
pub fn tap<T, E>(value: Result<T, E>, f: impl FnOnce(&T)) -> Result<T, E> {
    value.inspect(f)
}

/// Erase the success value (`void`).
pub fn void<T, E>(value: Result<T, E>) -> Result<(), E> {
    value.map(|_| ())
}

/// Run a throwing closure, catching panics (`try`).
pub fn try_<T>(f: impl FnOnce() -> T) -> Result<T, String> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(v) => Ok(v),
        Err(payload) => Err(payload
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| payload.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "unknown panic".to_string())),
    }
}

/// Unwrap or panic with the tag's `getOrThrow` message.
pub fn get_or_throw<T, E: std::fmt::Debug>(value: Result<T, E>) -> T {
    match value {
        Ok(v) => v,
        Err(e) => panic!("getOrThrow called on a Failure: {e:?}"),
    }
}

/// Unwrap or compute the fallback error.
pub fn get_or_throw_with<T, E, F>(
    value: Result<T, E>,
    on_failure: impl FnOnce(E) -> F,
) -> Result<T, F> {
    value.map_err(on_failure)
}

/// `null` encoding for failures (`getOrNull`/`tostring` shapes).
pub fn to_json<T: std::fmt::Display, E: std::fmt::Display>(value: &Result<T, E>) -> String {
    match value {
        Ok(v) => v.to_string(),
        Err(e) => format!("Failure({e})"),
    }
}
