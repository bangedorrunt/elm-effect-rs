//! `SchemaParser`: the adapter facade over a schema, and its ceiling.
//!
//! A schema whose steps are ordinary closures runs synchronously, so its decode
//! IS the exit. [`make`] and [`make_option`] are the construction adapters;
//! `decode_*` and `encode_*` are the decode and encode ones, each in the
//! carrier its name promises: [`Exit`], `Result`, `Option`, sync, effect.
//!
//! Every entry point has a `_with` twin taking the pin's parse options
//! (`ParseOptions`): `errors`, `onExcessProperty`, `disableChecks` and
//! `reportInput` decide what the parse reports, and a `_with` call's options are
//! scoped to that call. `Decoder`/`Encoder` are the creation-time spelling.
//!
//! # Effect steps
//!
//! `Schema::effect_codec` lets a node carry an Effect step instead of a
//! resolved closure. [`Effect`]'s adapter resolves those steps first
//! ([`resolve_steps`]) and decodes the answers second, so a product whose
//! children yield runs them in declaration order rather than running none.
//!
//! # Ceiling
//!
//! The resolution is sequential. The pin schedules siblings under
//! `ParseOptions.concurrency` with interruption of pending children and
//! completion-order accumulation; this port runs one step to completion before
//! starting the next, so the bounded-concurrency rows are excused on that
//! scheduler rather than on the schema. A union member or a record key that
//! carries a step is not resolved either: the walk only descends the shapes it
//! can name, and the second phase then reports the node's synchronous refusal
//! instead of a silently wrong value. The `transformEffect` spelling, whose
//! caller already ran the effect, still dies with [`ASYNC_DEFECT`].
//!
//! The mixed-decode-cause rows (a `Cause` holding an issue and a defect) are
//! excused on this port's decode having a single error channel, not on steps.

use elm_effect_schema::transform::{Errors, ParseOptions};
use elm_effect_schema::{
    AdapterError, CONSTRUCTOR_DIE, EffectStep, IntoUnknown, Schema, SchemaError, StepFuture, Unknown,
};

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

/// The effect adapter's `decode`.
///
/// A schema with no Effect step anywhere is decided here, so the result is an
/// `Effect` that has already settled. A schema that carries one is walked
/// first (see [`resolve_steps`]) and decoded second; a schema marked
/// `requires_effect` without a step graph (the `transformEffect` spelling, whose
/// caller already ran the effect) still dies with [`ASYNC_DEFECT`].
pub fn decode_unknown_effect<T>(schema: &Schema<T>, input: Unknown) -> Effect<T, SchemaError>
where
    T: Clone + Send + Sync + 'static,
{
    decode_unknown_effect_with(schema, input, ParseOptions::default())
}

/// [`decode_unknown_effect`] under parse options.
pub fn decode_unknown_effect_with<T>(
    schema: &Schema<T>,
    input: Unknown,
    options: ParseOptions,
) -> Effect<T, SchemaError>
where
    T: Clone + Send + Sync + 'static,
{
    if !schema.has_effect_steps() {
        return match decode_unknown_exit_with(schema, input, options) {
            Exit::Success(value) => Effect::succeed(value),
            Exit::Failure(cause) => Effect::fail_cause(cause),
        };
    }
    let schema = schema.clone();
    Effect::suspend(move || {
        let mut issues = Vec::new();
        let resolved = resolve_steps(&schema, input, "$", Direction::Decode, &options, &mut issues);
        let outcome = if issues.is_empty() {
            schema.decode_after_steps(resolved, &options)
        } else {
            Err(combined_issues(issues))
        };
        Effect::from_result(outcome)
    })
}

/// The effect adapter's `encode`.
///
/// The value is walked as its structural spelling, and every node that carries
/// an encode step answers the wire it produces; every other node keeps the
/// spelling it already has, which is the wire for the erased composites this
/// port builds.
pub fn encode_unknown_effect<T>(schema: &Schema<T>, value: &T) -> Effect<Unknown, SchemaError>
where
    T: Clone + IntoUnknown + Send + Sync + 'static,
{
    encode_unknown_effect_with(schema, value, ParseOptions::default())
}

/// [`encode_unknown_effect`] under parse options.
pub fn encode_unknown_effect_with<T>(
    schema: &Schema<T>,
    value: &T,
    options: ParseOptions,
) -> Effect<Unknown, SchemaError>
where
    T: Clone + IntoUnknown + Send + Sync + 'static,
{
    if !schema.has_effect_steps() {
        return match encode_unknown_exit_with(schema, value, options) {
            Exit::Success(encoded) => Effect::succeed(encoded),
            Exit::Failure(cause) => Effect::fail_cause(cause),
        };
    }
    let schema = schema.clone();
    let value = value.clone().into_unknown();
    Effect::suspend(move || {
        let mut issues = Vec::new();
        let resolved = resolve_steps(&schema, value, "$", Direction::Encode, &options, &mut issues);
        Effect::from_result(if issues.is_empty() {
            Ok(resolved)
        } else {
            Err(combined_issues(issues))
        })
    })
}

/// Which of a node's two codecs the walk runs.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Direction {
    Decode,
    Encode,
}

impl Direction {
    fn step<'a, T>(self, schema: &'a Schema<T>) -> Option<&'a dyn EffectStep> {
        schema.effect_steps().map(|(decode, encode)| match self {
            Self::Decode => decode,
            Self::Encode => encode,
        })
    }
}

/// Run every Effect step under `schema`, replacing each node's input with the
/// step's answer, and hand the resulting input to the ordinary decoder.
///
/// This is the pin's sequential parser for the shapes it can see here: a struct
/// walks its declared fields, a tuple its positions, an array its elements. A
/// union or a record is left unresolved, and the second phase then reports the
/// node's synchronous refusal rather than a silent value.
///
/// A step's own failure becomes an issue at the occurrence it was reading,
/// `errors: \"first\"` stops at it, and `errors: \"all\"` keeps walking the
/// siblings; both are the composite rules the exit adapter already applies.
fn resolve_steps<T>(
    schema: &Schema<T>,
    input: Unknown,
    path: &str,
    direction: Direction,
    options: &ParseOptions,
    issues: &mut Vec<SchemaError>,
) -> Unknown
where
    T: Send + Sync + 'static,
{
    if let Some(step) = direction.step(schema) {
        return match block_on(step.run(input)) {
            Ok(answer) => answer,
            Err(error) => {
                issues.push(error.at(path));
                Unknown::Undefined
            }
        };
    }
    // A node with no step below it is left exactly as it arrived, so the
    // ordinary decoder does all of its own work.
    if !schema.has_effect_steps() {
        return input;
    }
    match input {
        Unknown::Object(mut object) => {
            for field in schema.fields().unwrap_or_default() {
                let Some(value) = object.remove(field.name()) else {
                    continue;
                };
                if !field.inner().has_effect_steps() {
                    object.insert(field.name().to_string(), value);
                    continue;
                }
                let child = format!("{path}.{}", field.name());
                let resolved =
                    resolve_steps(field.inner(), value, &child, direction, options, issues);
                object.insert(field.name().to_string(), resolved);
                if options.errors == Errors::First && !issues.is_empty() {
                    return Unknown::Object(object);
                }
            }
            // A record names one value schema for every key its index
            // signature covers; an exact-key record walks the keys it holds.
            if schema.fields().is_none()
                && let Some(value_schema) = schema.record_value()
                && value_schema.has_effect_steps()
            {
                for (name, value) in std::mem::take(&mut object) {
                    let child = format!("{path}.{name}");
                    let resolved =
                        resolve_steps(value_schema, value, &child, direction, options, issues);
                    object.insert(name, resolved);
                    if options.errors == Errors::First && !issues.is_empty() {
                        return Unknown::Object(object);
                    }
                }
            }
            Unknown::Object(object)
        }
        Unknown::Array(mut items) => {
            // A tuple names its positions; an array names one element schema
            // for every position. Both are walked the same way.
            let elements: Vec<Schema<Unknown>> = match schema.elements() {
                Some(elements) => elements.to_vec(),
                None => schema
                    .array_element_schema()
                    .map(|element| vec![element.clone(); items.len()])
                    .unwrap_or_default(),
            };
            for (index, element) in elements.iter().enumerate() {
                let Some(value) = items.get_mut(index) else {
                    continue;
                };
                let taken = std::mem::replace(value, Unknown::Undefined);
                if !element.has_effect_steps() {
                    *value = taken;
                    continue;
                }
                let child = format!("{path}[{index}]");
                *value = resolve_steps(element, taken, &child, direction, options, issues);
                if options.errors == Errors::First && !issues.is_empty() {
                    return Unknown::Array(items);
                }
            }
            Unknown::Array(items)
        }
        other => other,
    }
}

/// One issue is that issue; more than one is the composite the exit adapter
/// already reports.
fn combined_issues(mut issues: Vec<SchemaError>) -> SchemaError {
    if issues.len() == 1 {
        issues.pop().expect("one issue")
    } else {
        SchemaError::Composite { issues }
    }
}

/// Drive one step's future to completion on this thread.
///
/// A fiber body already owns a thread and never shares it, so parking here is
/// the same wait the fiber itself would do. The waker unparks this thread, which
/// is all a step needs to make progress.
fn block_on(mut future: StepFuture) -> Result<Unknown, SchemaError> {
    use std::sync::Arc;
    use std::task::{Context, Poll, Wake, Waker};

    struct Unpark(std::thread::Thread);

    impl Wake for Unpark {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }

    let waker = Waker::from(Arc::new(Unpark(std::thread::current())));
    let mut context = Context::from_waker(&waker);
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(answer) => return answer,
            Poll::Pending => std::thread::park(),
        }
    }
}

/// `SchemaParser.decodeUnknownExit(schema)(input)`: success, a typed failure,
/// or a defect for a schema this adapter cannot run.
pub fn decode_unknown_exit<T>(
    schema: &Schema<T>,
    input: Unknown,
) -> Exit<T, SchemaError, Defect> {
    decode_unknown_exit_with(schema, input, ParseOptions::default())
}

/// [`decode_unknown_exit`] with the pin's per-call options.
pub fn decode_unknown_exit_with<T>(
    schema: &Schema<T>,
    input: Unknown,
    options: ParseOptions,
) -> Exit<T, SchemaError, Defect> {
    if schema.requires_effect() {
        return Exit::die(Defect::new(ASYNC_DEFECT));
    }
    match schema.decode_with(input, &options) {
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
    encode_unknown_exit_with(schema, value, ParseOptions::default())
}

/// [`encode_unknown_exit`] with the pin's per-call options.
pub fn encode_unknown_exit_with<T>(
    schema: &Schema<T>,
    value: &T,
    options: ParseOptions,
) -> Exit<Unknown, SchemaError, Defect> {
    if schema.requires_effect() {
        return Exit::die(Defect::new(ASYNC_DEFECT));
    }
    match schema.encode_with(value, &options) {
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
    decode_unknown_result_with(schema, input, ParseOptions::default())
}

/// [`decode_unknown_result`] under parse options.
pub fn decode_unknown_result_with<T>(
    schema: &Schema<T>,
    input: Unknown,
    options: ParseOptions,
) -> Result<Result<T, SchemaError>, AdapterRefusal<SchemaError>> {
    decode_unknown_exit_with(schema, input, options).into_result()
}

/// `SchemaParser.encodeUnknownResult(schema)(value)`.
pub fn encode_unknown_result<T>(
    schema: &Schema<T>,
    value: &T,
) -> Result<Result<Unknown, SchemaError>, AdapterRefusal<SchemaError>> {
    encode_unknown_result_with(schema, value, ParseOptions::default())
}

/// [`encode_unknown_result`] under parse options.
pub fn encode_unknown_result_with<T>(
    schema: &Schema<T>,
    value: &T,
    options: ParseOptions,
) -> Result<Result<Unknown, SchemaError>, AdapterRefusal<SchemaError>> {
    encode_unknown_exit_with(schema, value, options).into_result()
}

/// `SchemaParser.decodeUnknownOption(schema)(input)`: `Some` for a success,
/// `None` for a bare issue.
pub fn decode_unknown_option<T>(
    schema: &Schema<T>,
    input: Unknown,
) -> Result<Option<T>, AdapterRefusal<SchemaError>> {
    decode_unknown_option_with(schema, input, ParseOptions::default())
}

/// [`decode_unknown_option`] under parse options.
pub fn decode_unknown_option_with<T>(
    schema: &Schema<T>,
    input: Unknown,
    options: ParseOptions,
) -> Result<Option<T>, AdapterRefusal<SchemaError>> {
    decode_unknown_exit_with(schema, input, options).into_option()
}

/// `SchemaParser.encodeUnknownOption(schema)(value)`.
pub fn encode_unknown_option<T>(
    schema: &Schema<T>,
    value: &T,
) -> Result<Option<Unknown>, AdapterRefusal<SchemaError>> {
    encode_unknown_option_with(schema, value, ParseOptions::default())
}

/// [`encode_unknown_option`] under parse options.
pub fn encode_unknown_option_with<T>(
    schema: &Schema<T>,
    value: &T,
    options: ParseOptions,
) -> Result<Option<Unknown>, AdapterRefusal<SchemaError>> {
    encode_unknown_exit_with(schema, value, options).into_option()
}

/// `SchemaParser.decodeUnknownSync(schema)(input)`: the value, or what the sync
/// adapter throws. A `Result` is this port's throw; the message is the parser
/// file's wording.
pub fn decode_unknown_sync<T>(
    schema: &Schema<T>,
    input: Unknown,
) -> Result<T, AdapterFailure<SchemaError>> {
    decode_unknown_sync_with(schema, input, ParseOptions::default())
}

/// [`decode_unknown_sync`] under parse options.
pub fn decode_unknown_sync_with<T>(
    schema: &Schema<T>,
    input: Unknown,
    options: ParseOptions,
) -> Result<T, AdapterFailure<SchemaError>> {
    decode_unknown_exit_with(schema, input, options).into_value_with(SYNC_ISSUE_MESSAGE)
}

/// `SchemaParser.encodeUnknownSync(schema)(value)`.
pub fn encode_unknown_sync<T>(
    schema: &Schema<T>,
    value: &T,
) -> Result<Unknown, AdapterFailure<SchemaError>> {
    encode_unknown_sync_with(schema, value, ParseOptions::default())
}

/// [`encode_unknown_sync`] under parse options.
pub fn encode_unknown_sync_with<T>(
    schema: &Schema<T>,
    value: &T,
    options: ParseOptions,
) -> Result<Unknown, AdapterFailure<SchemaError>> {
    encode_unknown_exit_with(schema, value, options).into_value_with(SYNC_ISSUE_MESSAGE)
}

/// `SchemaParser.make(schema)(input)`: construction, not decode. A present
/// nested composite is constructed too, so constructor defaults fill
/// recursively.
///
/// A default that died is not a schema issue, so it arrives as
/// [`AdapterError::Die`] with the adapter's own message, the defect, and any
/// issue that rode beside it in the same cause.
pub fn make<T>(schema: &Schema<T>, input: Unknown) -> Result<T, AdapterError> {
    make_with(schema, input, ParseOptions::default())
}

/// [`make`] under parse options.
pub fn make_with<T>(
    schema: &Schema<T>,
    input: Unknown,
    options: ParseOptions,
) -> Result<T, AdapterError> {
    schema
        .construct_with(input, &options)
        .map_err(|error| AdapterError::report(error, CONSTRUCTOR_DIE))
}

/// `SchemaParser.makeOption(schema)(input)`: construction as an `Option`. A
/// schema issue answers `Ok(None)`; a died default is a refusal, never `None`.
pub fn make_option<T>(schema: &Schema<T>, input: Unknown) -> Result<Option<T>, AdapterError> {
    make_option_with(schema, input, ParseOptions::default())
}

/// [`make_option`] under parse options.
pub fn make_option_with<T>(
    schema: &Schema<T>,
    input: Unknown,
    options: ParseOptions,
) -> Result<Option<T>, AdapterError> {
    options.scoped(|| schema.make_option(input))
}

/// The throw the pin's sync adapter performs. Re-exported here because the
/// message is what a caller matching on the refusal reads.
pub fn sync_message() -> &'static str {
    crate::schema_adapter::SYNC_MESSAGE
}
