//! Unstable adapters behind cargo features. Nothing here loads unless its
//! feature is enabled; stable surfaces never depend on this crate.

#[cfg(feature = "otel")]
pub mod otel;

pub mod workflow;

#[cfg(feature = "eventlog")]
pub mod eventlog;

#[cfg(feature = "persistence")]
pub mod persistence;

pub mod cli;

pub mod ai;

#[cfg(feature = "schema_obs")]
pub mod schema_obs;

pub mod http;

pub mod process;

pub mod workers;

pub mod reactivity;

pub mod devtools;
