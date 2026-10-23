//! A typed error that names its own kind (`Data.TaggedError`'s `_tag`).
//!
//! The pin reads a `_tag` field off the error value at runtime; this port asks
//! the value instead, through [`TaggedError`], so
//! [`Layer::catch_tag`](crate::Layer::catch_tag) can recover one tag and let
//! every other cause through untouched.
//!
//! A tag belongs to the error's kind, but the kind is not always the type: one
//! enum may carry several tags, and only the value knows which one it holds.
//! That is why this is a method and not an associated constant.

/// An error that can say which kind it is.
///
/// Implement it for the error type a `Layer` or `Effect` fails with, then name
/// the tag in [`Layer::catch_tag`](crate::Layer::catch_tag). Defects and
/// interruptions never reach this trait: they are not typed failures.
pub trait TaggedError {
    /// This value's tag, e.g. `"LayerError"`.
    fn tag(&self) -> &'static str;
}
