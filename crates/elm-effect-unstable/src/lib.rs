//! Unstable adapters behind cargo features. Nothing here loads unless its
//! feature is enabled; stable surfaces never depend on this crate.

#[cfg(feature = "otel")]
pub mod otel;
