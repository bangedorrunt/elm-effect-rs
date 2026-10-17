//! Unstable adapters behind cargo features. Nothing here loads unless its
//! feature is enabled; stable surfaces never depend on this crate.

#[cfg(feature = "otel")]
pub mod otel;

pub mod workflow;

#[cfg(feature = "eventlog")]
pub mod eventlog;
#[cfg(feature = "eventlog")]
pub mod eventlog_identity;
#[cfg(feature = "eventlog")]
pub mod eventlog_session_auth;

#[cfg(feature = "persistence")]
pub mod persistence;

pub mod cli;

pub mod cli_completions;

pub mod ai;

pub mod cli_ai;

#[cfg(feature = "schema_obs")]
pub mod schema_obs;

pub mod http;
pub mod http_platform;
pub mod net;
pub mod mime;

pub mod http_server;
pub mod http_endpoint;
pub mod http_docs;

pub mod process;

pub mod workers;

pub mod reactivity;
pub mod mcp_schema;

pub mod devtools;

pub mod arbitrary;

pub mod schema_binary;
