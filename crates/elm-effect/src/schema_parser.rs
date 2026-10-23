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
//! ([`resolve`]) and decodes the answers second, so a product whose children
//! yield gets every step's answer without a sync adapter ever pretending to run
//! one.
//!
//! # Scheduling
//!
//! A product fans its step-carrying children out under
//! `ParseOptions.concurrency`: `bounded(n)` keeps `n` open and starts the next
//! as a slot frees, `unbounded` starts them all, and the default sequential runs
//! one at a time in declaration order. Each child reads the same option for its
//! own children, which is the pin's nesting rule. Under `errors: first` the
//! first failure to complete wins and the children still open are dropped, the
//! carrier's interruption; under `all` the issues accumulate in completion
//! order, and a record assembles its keys in that order too.
//!
//! # Ceiling
//!
//! A union member that carries a step is not resolved: the walk only descends
//! the shapes it can name, and the second phase then reports the node's
//! synchronous refusal instead of a silently wrong value. A constructor default
//! fills in that second phase as well, so the concurrency knob has no default to
//! schedule. The `transformEffect` spelling, whose caller already ran the
//! effect, still dies with [`ASYNC_DEFECT`].
//!
//! The mixed-decode-cause rows (a `Cause` holding an issue and a defect) are
//! excused on this port's decode having a single error channel, not on steps.

use elm_effect_schema::transform::{Errors, ParseOptions};
use elm_effect_schema::{
    AdapterError, CONSTRUCTOR_DIE, EffectStep, IntoUnknown, Schema, SchemaError, Unknown,
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
        let resolved = block_on(resolve(
            schema.clone(),
            input,
            "$".to_string(),
            Direction::Decode,
            options,
        ));
        let outcome = match resolved {
            Ok(input) => schema.decode_after_steps(input, &options),
            Err(error) => Err(error),
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
        let resolved = block_on(resolve(
            schema.clone(),
            value,
            "$".to_string(),
            Direction::Encode,
            options,
        ));
        Effect::from_result(resolved)
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

/// One node's resolved value, or the issue that stopped it.
type ResolveFuture = std::pin::Pin<
    Box<dyn std::future::Future<Output = Result<Unknown, SchemaError>> + Send + 'static>,
>;

/// Resolve every Effect step under `schema`, replacing each node's input with
/// the step's answer.
///
/// A product fans its step-carrying children out under
/// `ParseOptions.concurrency`, and each child that is itself a product fans out
/// under the same option independently: that is the pin's nesting rule
/// (`SchemaParser.test.ts:247`). `errors: \"first\"` returns the first failure
/// that completes and drops every child still open, which is this carrier's
/// interruption; `\"all\"` reports the failures in completion order.
fn resolve<T>(
    schema: Schema<T>,
    input: Unknown,
    path: String,
    direction: Direction,
    options: ParseOptions,
) -> ResolveFuture
where
    T: Clone + Send + Sync + 'static,
{
    Box::pin(async move {
        // A node that carries the step answers with it; its children are the
        // step's own business.
        if let Some(step) = direction.step(&schema) {
            let future = step.run(input);
            return future.await.map_err(|error| error.at(&path));
        }
        // A node with no step below it is left exactly as it arrived, so the
        // ordinary decoder does all of its own work.
        if !schema.has_effect_steps() {
            return Ok(input);
        }
        match input {
            Unknown::Object(object) => {
                resolve_object(&schema, object, path, direction, options).await
            }
            Unknown::Array(items) => {
                resolve_array(&schema, items, path, direction, options).await
            }
            other => Ok(other),
        }
    })
}

/// The object shapes: a struct walks its declared fields, a record walks every
/// entry under its one value schema.
fn resolve_object<T>(
    schema: &Schema<T>,
    object: std::collections::BTreeMap<String, Unknown>,
    path: String,
    direction: Direction,
    options: ParseOptions,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Unknown, SchemaError>> + Send + '_>>
where
    T: Clone + Send + Sync + 'static,
{
    Box::pin(async move {
        let limit = options.concurrency.slots();
        let fail_fast = options.errors == Errors::First;
        if let Some(fields) = schema.fields() {
            let mut names = Vec::new();
            let mut tasks = Vec::new();
            for field in fields {
                let Some(value) = object.get(field.name()) else {
                    continue;
                };
                if !field.inner().has_effect_steps() {
                    continue;
                }
                names.push(field.name().to_string());
                tasks.push(resolve(
                    field.inner().clone(),
                    value.clone(),
                    format!("{path}.{}", field.name()),
                    direction,
                    options,
                ));
            }
            let done = FanOut::new(tasks, limit, fail_fast).await?;
            let mut out = object;
            for (index, value) in done {
                out.insert(names[index].clone(), value);
            }
            return Ok(Unknown::Object(out));
        }
        // A record names one value schema for every key its index signature
        // covers. Entries are assembled in COMPLETION order, so two wire keys
        // that decode onto one name are decided the way the pin decides them
        // (`SchemaParser.test.ts:221`).
        let Some(value_schema) = schema.record_value() else {
            return Ok(Unknown::Object(object));
        };
        if !value_schema.has_effect_steps() {
            return Ok(Unknown::Object(object));
        }
        let keys: Vec<String> = object.keys().cloned().collect();
        let tasks = keys
            .iter()
            .map(|key| {
                resolve(
                    value_schema.clone(),
                    object[key].clone(),
                    format!("{path}.{key}"),
                    direction,
                    options,
                )
            })
            .collect();
        let done = FanOut::new(tasks, limit, fail_fast).await?;
        let mut out = std::collections::BTreeMap::new();
        for (index, value) in done {
            let wire_key = &keys[index];
            let name = match schema.record_key() {
                Some(key_schema) => match key_schema.decode(Unknown::Text(wire_key.clone())) {
                    Ok(Unknown::Text(name)) => name,
                    _ => wire_key.clone(),
                },
                None => wire_key.clone(),
            };
            // Last completion wins, which is the pin's rule for a collision.
            out.insert(name, value);
        }
        Ok(Unknown::Object(out))
    })
}

/// The positional shapes: a tuple names each position, an array names one
/// element schema for every position.
fn resolve_array<T>(
    schema: &Schema<T>,
    items: Vec<Unknown>,
    path: String,
    direction: Direction,
    options: ParseOptions,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Unknown, SchemaError>> + Send + '_>>
where
    T: Clone + Send + Sync + 'static,
{
    Box::pin(async move {
        let limit = options.concurrency.slots();
        let fail_fast = options.errors == Errors::First;
        let elements: Vec<Schema<Unknown>> = match schema.elements() {
            Some(elements) => elements.to_vec(),
            None => schema
                .array_element_schema()
                .map(|element| vec![element.clone(); items.len()])
                .unwrap_or_default(),
        };
        let mut positions = Vec::new();
        let mut tasks = Vec::new();
        let mut taken = items;
        for (index, element) in elements.iter().enumerate() {
            if !element.has_effect_steps() || index >= taken.len() {
                continue;
            }
            positions.push(index);
            let value = std::mem::replace(&mut taken[index], Unknown::Undefined);
            tasks.push(resolve(
                element.clone(),
                value,
                format!("{path}[{index}]"),
                direction,
                options,
            ));
        }
        let done = FanOut::new(tasks, limit, fail_fast).await?;
        for (task, value) in done {
            taken[positions[task]] = value;
        }
        Ok(Unknown::Array(taken))
    })
}

/// Poll a product's children under a slot limit, answering in completion order.
///
/// Every child is polled with the parent's waker, so one child waking makes the
/// parent re-poll all of them. `ponytail: O(n²)` re-poll over a product's
/// children; a product is the only place this runs and `n` is its arity, so an
/// intrusive waker list would be more machinery than it saves.
struct FanOut {
    pending: std::collections::VecDeque<(usize, ResolveFuture)>,
    active: Vec<(usize, ResolveFuture)>,
    completed: Vec<(usize, Result<Unknown, SchemaError>)>,
    limit: usize,
    fail_fast: bool,
}

impl FanOut {
    fn new(tasks: Vec<ResolveFuture>, limit: usize, fail_fast: bool) -> Self {
        Self {
            pending: tasks.into_iter().enumerate().collect(),
            active: Vec::new(),
            completed: Vec::new(),
            limit,
            fail_fast,
        }
    }
}

impl std::future::Future for FanOut {
    type Output = Result<Vec<(usize, Unknown)>, SchemaError>;

    fn poll(
        self: std::pin::Pin<&mut Self>,
        context: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        let this = self.get_mut();
        loop {
            while this.active.len() < this.limit {
                match this.pending.pop_front() {
                    Some(task) => this.active.push(task),
                    None => break,
                }
            }
            if this.active.is_empty() {
                let mut values = Vec::new();
                let mut issues = Vec::new();
                for (index, result) in this.completed.drain(..) {
                    match result {
                        Ok(value) => values.push((index, value)),
                        Err(error) => issues.push(error),
                    }
                }
                return std::task::Poll::Ready(if issues.is_empty() {
                    Ok(values)
                } else {
                    Err(combined_issues(issues))
                });
            }
            let mut progressed = false;
            let mut index = 0;
            while index < this.active.len() {
                let (task, mut future) = this.active.remove(index);
                match future.as_mut().poll(context) {
                    std::task::Poll::Ready(result) => {
                        match result {
                            Ok(value) => this.completed.push((task, Ok(value))),
                            Err(error) if this.fail_fast => {
                                // Dropping the open children is this carrier's
                                // interruption (`SchemaParser.test.ts:137`).
                                this.active.clear();
                                this.pending.clear();
                                return std::task::Poll::Ready(Err(error));
                            }
                            Err(error) => this.completed.push((task, Err(error))),
                        }
                        progressed = true;
                    }
                    std::task::Poll::Pending => {
                        this.active.insert(index, (task, future));
                        index += 1;
                    }
                }
            }
            if !progressed {
                return std::task::Poll::Pending;
            }
        }
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

/// Drive a resolver future to completion on this thread.
///
/// A fiber body already owns a thread and never shares it, so parking here is
/// the same wait the fiber itself would do. The waker unparks this thread, which
/// is all a step or a child product needs to make progress.
fn block_on<F, R>(future: F) -> R
where
    F: std::future::Future<Output = R>,
{
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
    let mut future = std::pin::pin!(future);
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
