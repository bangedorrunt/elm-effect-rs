//! `SchemaParser.decodeUnknownExit` / `decodeUnknownResult`: the sync adapters.
//!
//! A schema whose steps are all ordinary closures runs synchronously, so its
//! decode IS the exit. A schema carrying an Effect step (`transformEffect`,
//! `checkEffect`) needs a runtime this adapter does not have, and the pin's
//! answer is to die rather than block or swallow: `decodeUnknownExit` reports a
//! defect, and `decodeUnknownResult` throws the sync adapter's refusal because
//! a `Result` cannot carry a die.
//!
//! The Effect step itself is run by whoever resolved it and handed the schema a
//! closure, which is why a plain `decode` of the same schema still works
//! (`Schema.test.ts:6195`, `:8323`).

use elm_effect_schema::{Schema, SchemaError, Unknown};

use crate::cause::Defect;
use crate::exit::Exit;
use crate::schema_adapter::{AdapterRefusal, SYNC_MESSAGE};

/// The defect a sync adapter reports when a schema needs an Effect runtime.
pub const ASYNC_DEFECT: &str = "Sync adapter cannot run an async schema";

/// `SchemaParser.decodeUnknownExit(schema)(input)`: success, a typed failure,
/// or a defect for a schema this adapter cannot run.
pub fn decode_unknown_exit<T>(
    schema: &Schema<T>,
    input: Unknown,
) -> Exit<T, SchemaError, Defect> {
    if schema.requires_effect() {
        return Exit::die(Defect::new(ASYNC_DEFECT));
    }
    match schema.decode(input) {
        Ok(value) => Exit::succeed(value),
        Err(error) => Exit::fail(error),
    }
}

/// `SchemaParser.decodeUnknownResult(schema)(input)`: the schema's own issue
/// inside the `Result`, or the adapter's refusal (the throw) when the schema
/// needs an Effect runtime or a step died.
pub fn decode_unknown_result<T>(
    schema: &Schema<T>,
    input: Unknown,
) -> Result<Result<T, SchemaError>, AdapterRefusal<SchemaError>> {
    decode_unknown_exit(schema, input).into_result()
}

/// The throw the pin's sync adapter performs. Re-exported here because the
/// message is what a caller matching on the refusal reads.
pub fn sync_message() -> &'static str {
    SYNC_MESSAGE
}
