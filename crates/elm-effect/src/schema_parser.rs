//! `SchemaParser`: the adapter facade over a schema, and its ceiling.
//!
//! A schema whose steps are ordinary closures runs synchronously, so its decode
//! IS the exit. [`make`] and [`make_option`] are the construction adapters;
//! `decode_*` and `encode_*` are the decode and encode ones, each in the
//! carrier its name promises: [`Exit`], `Result`, `Option`, sync, effect.
//!
//! # Ceiling
//!
//! The pin's parser runs Effect-carrying schema steps and schedules them:
//! bounded concurrency, interruption of pending children, completion-order
//! accumulation, and lazily compiled parsers. This port's schema resolves each
//! step before it is attached. `decode_to_effect` and `checked_effect` mark
//! `requires_effect` and carry the already-resolved transformation, so there
//! is no step graph left for a parser to schedule. The effect adapter refuses
//! such a schema with [`ASYNC_DEFECT`] rather than pretending to run it.
//!
//! Porting those rows means `Schema<T>` itself has to carry the effect: a
//! schema redesign, not a parser file. The concurrency, mixed-decode-cause,
//! and async-middleware rows are excused on that mechanism.

use elm_effect_schema::{AdapterError, CONSTRUCTOR_DIE, Schema, SchemaError, Unknown};

use crate::cause::Defect;
use crate::effect::Effect;
use crate::exit::Exit;
use crate::schema_adapter::{AdapterFailure, AdapterRefusal};

/// The defect a sync adapter reports when a schema needs an Effect runtime.
pub const ASYNC_DEFECT: &str = "Sync adapter cannot run an async schema";

/// The sync adapter's wording in `SchemaParser.test.ts` (`:543`).
///
/// The pin's own two test files disagree: `Schema.test.ts:8635` asserts
/// "schema errors" for the same adapter. Each entry point here carries the
/// wording its own file asserts, so [`Exit::into_value`] keeps the
/// `Schema.test.ts` text and this function takes the parser's.
pub const SYNC_ISSUE_MESSAGE: &str = "Sync adapter can only throw schema issues";

/// The effect adapter's `decode` for a schema whose steps are closures.
///
/// A sync schema is decided here, so the result is an `Effect` that has already
/// settled: a success, or the same cause the exit carries. A schema marked
/// `requires_effect` dies with [`ASYNC_DEFECT`].
pub fn decode_unknown_effect<T>(schema: &Schema<T>, input: Unknown) -> Effect<T, SchemaError>
where
    T: Send + 'static,
{
    match decode_unknown_exit(schema, input) {
        Exit::Success(value) => Effect::succeed(value),
        Exit::Failure(cause) => Effect::fail_cause(cause),
    }
}

/// The effect adapter's `encode`, with the same ceiling as
/// [`decode_unknown_effect`].
pub fn encode_unknown_effect<T>(schema: &Schema<T>, value: &T) -> Effect<Unknown, SchemaError>
where
    T: Send + 'static,
{
    match encode_unknown_exit(schema, value) {
        Exit::Success(encoded) => Effect::succeed(encoded),
        Exit::Failure(cause) => Effect::fail_cause(cause),
    }
}

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

/// `SchemaParser.encodeUnknownExit(schema)(value)`: the wire value, or the
/// refusal the encoder reported.
pub fn encode_unknown_exit<T>(
    schema: &Schema<T>,
    value: &T,
) -> Exit<Unknown, SchemaError, Defect> {
    if schema.requires_effect() {
        return Exit::die(Defect::new(ASYNC_DEFECT));
    }
    match schema.try_encode(value) {
        Ok(encoded) => Exit::succeed(encoded),
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

/// `SchemaParser.encodeUnknownResult(schema)(value)`.
pub fn encode_unknown_result<T>(
    schema: &Schema<T>,
    value: &T,
) -> Result<Result<Unknown, SchemaError>, AdapterRefusal<SchemaError>> {
    encode_unknown_exit(schema, value).into_result()
}

/// `SchemaParser.decodeUnknownOption(schema)(input)`: `Some` for a success,
/// `None` for a bare issue.
pub fn decode_unknown_option<T>(
    schema: &Schema<T>,
    input: Unknown,
) -> Result<Option<T>, AdapterRefusal<SchemaError>> {
    decode_unknown_exit(schema, input).into_option()
}

/// `SchemaParser.encodeUnknownOption(schema)(value)`.
pub fn encode_unknown_option<T>(
    schema: &Schema<T>,
    value: &T,
) -> Result<Option<Unknown>, AdapterRefusal<SchemaError>> {
    encode_unknown_exit(schema, value).into_option()
}

/// `SchemaParser.decodeUnknownSync(schema)(input)`: the value, or what the sync
/// adapter throws. A `Result` is this port's throw; the message is the parser
/// file's wording.
pub fn decode_unknown_sync<T>(
    schema: &Schema<T>,
    input: Unknown,
) -> Result<T, AdapterFailure<SchemaError>> {
    decode_unknown_exit(schema, input).into_value_with(SYNC_ISSUE_MESSAGE)
}

/// `SchemaParser.encodeUnknownSync(schema)(value)`.
pub fn encode_unknown_sync<T>(
    schema: &Schema<T>,
    value: &T,
) -> Result<Unknown, AdapterFailure<SchemaError>> {
    encode_unknown_exit(schema, value).into_value_with(SYNC_ISSUE_MESSAGE)
}

/// `SchemaParser.make(schema)(input)`: construction, not decode. A present
/// nested composite is constructed too, so constructor defaults fill
/// recursively.
///
/// A default that died is not a schema issue, so it arrives as
/// [`AdapterError::Die`] with the adapter's own message, the defect, and any
/// issue that rode beside it in the same cause.
pub fn make<T>(schema: &Schema<T>, input: Unknown) -> Result<T, AdapterError> {
    schema
        .construct(input)
        .map_err(|error| AdapterError::report(error, CONSTRUCTOR_DIE))
}

/// `SchemaParser.makeOption(schema)(input)`: construction as an `Option`. A
/// schema issue answers `Ok(None)`; a died default is a refusal, never `None`.
pub fn make_option<T>(schema: &Schema<T>, input: Unknown) -> Result<Option<T>, AdapterError> {
    schema.make_option(input)
}

/// The throw the pin's sync adapter performs. Re-exported here because the
/// message is what a caller matching on the refusal reads.
pub fn sync_message() -> &'static str {
    crate::schema_adapter::SYNC_MESSAGE
}
