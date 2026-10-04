//! `Utils` port (`Utils.ts` at the pin): the portable piece.
//!
//! Residuals (JS-runtime machinery, no Rust spelling): `Variance` / `Gen`
//! (type-level), `internalCall` (stack-trace capture).

/// Single-shot iterator: yields the stored value once, then ends.
/// Mirrors `SingleShotGen` (the value sent back on completion has no Rust
/// spelling; the iterator simply ends).
#[derive(Debug)]
pub struct SingleShotGen<T> {
    value: Option<T>,
}

impl<T> SingleShotGen<T> {
    /// Wrap `value` for one yield.
    pub fn new(value: T) -> Self {
        Self { value: Some(value) }
    }
}

impl<T> Iterator for SingleShotGen<T> {
    type Item = T;

    fn next(&mut self) -> Option<T> {
        self.value.take()
    }
}
