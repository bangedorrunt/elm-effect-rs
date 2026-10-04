//! `Brand.ts` port: nominal and refined branded types.
//!
//! A brand is a phantom marker: zero runtime cost, distinct type identity.
//! Constructors: [`nominal`] (no validation), [`refined`] (one predicate),
//! [`validate`] (many checks with accumulated failures and abort).

use std::marker::PhantomData;

/// A value of type `T` carrying brand `B`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Branded<T, B> {
    value: T,
    brand: PhantomData<fn() -> B>,
}

impl<T, B> Branded<T, B> {
    /// The underlying value.
    pub fn value(&self) -> &T {
        &self.value
    }

    /// Unwrap to the underlying value.
    pub fn into_value(self) -> T {
        self.value
    }
}

/// One schema-style check: pass, fail-and-continue, or fail-and-abort.
pub enum Check {
    Pass,
    Fail(String),
    Abort(String),
}

/// A validation failure, formatted like the tag's `BrandError`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BrandError(pub String);

impl std::fmt::Display for BrandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "BrandError: {}", self.0)
    }
}

impl std::error::Error for BrandError {}

/// Create a nominal brand: no runtime validation.
pub fn nominal<T, B>(value: T) -> Branded<T, B> {
    Branded {
        value,
        brand: PhantomData,
    }
}

/// Create a brand behind one custom predicate (`make`).
pub fn make<T, B>(
    value: T,
    predicate: impl FnOnce(&T) -> bool,
    name: &str,
) -> Result<Branded<T, B>, BrandError> {
    if predicate(&value) {
        Ok(nominal(value))
    } else {
        Err(BrandError(format!("predicate {name} failed")))
    }
}

/// Validate one schema check.
pub fn refined<T, B>(
    value: T,
    check: impl FnOnce(&T) -> Check,
) -> Result<Branded<T, B>, Vec<BrandError>> {
    match check(&value) {
        Check::Pass => Ok(nominal(value)),
        Check::Fail(message) | Check::Abort(message) => Err(vec![BrandError(message)]),
    }
}

/// Combine constructors (`Brand.all`): every check runs in order, failures
/// accumulate, and an abort stops the rest.
pub fn validate<T, B>(
    value: T,
    checks: &[Box<dyn Fn(&T) -> Check>],
) -> Result<Branded<T, B>, Vec<BrandError>> {
    let mut failures = Vec::new();
    for check in checks {
        match check(&value) {
            Check::Pass => {}
            Check::Fail(message) => failures.push(BrandError(message)),
            Check::Abort(message) => {
                failures.push(BrandError(message));
                break;
            }
        }
    }
    if failures.is_empty() {
        Ok(nominal(value))
    } else {
        Err(failures)
    }
}
