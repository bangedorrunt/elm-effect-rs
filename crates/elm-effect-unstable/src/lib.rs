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
/// `Schema.URLFromString`, beside the `Url` value it speaks for.
pub mod schema_url;
pub use schema_url::{url_from_string, url_wire};
pub mod http_compression;
pub mod http_platform;
pub mod mime;
pub mod net;

pub mod http_docs;
pub mod http_endpoint;
pub mod http_server;
pub mod openapi;

pub mod process;

pub mod workers;

pub mod mcp_assembly;
pub mod mcp_handlers;
pub mod mcp_protocol;
pub mod mcp_schema;
pub mod mcp_server;
pub mod prompt;
pub mod reactivity;

pub mod devtools;

pub mod arbitrary;

pub mod schema_binary;
