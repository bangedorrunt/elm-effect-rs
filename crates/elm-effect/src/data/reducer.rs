//! `Reducer.ts` port: re-exported from [`combiner`](mod@combiner).
//!
//! Self-contained shim like the `.15` cron/datetime pair: the test target
//! includes this file by path, which pulls the implementation the same way.
//! The follow-up is one `mod`/`pub use` pair plus dropping this shim.

#[path = "combiner.rs"]
mod combiner;

pub use combiner::{Reducer, concat_string_reducer};
