//! The pin's sequential effect parser: a product whose children are Effect
//! steps, resolved in declaration order on both axes.
//!
//! `SchemaParser.test.ts:45` drives four product shapes through two error modes
//! and asserts the call log is `start a`, `end a`, `start b`, `end b` for
//! decoding and for encoding. The step here yields once (answers `Pending` and
//! wakes itself) so "sequential" is observable without a second thread.

#![cfg(all(feature = "schema", feature = "tokio"))]

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use elm_effect::{
    Exit, Runtime, decode_unknown_effect, decode_unknown_effect_with, decode_unknown_exit,
    decode_unknown_sync, encode_unknown_effect_with,
};
use elm_effect_schema::transform::{Errors, ParseOptions};
use elm_effect_schema::{Field, Schema, SchemaError, StepCause, StepFuture, Unknown};

fn text(value: &str) -> Unknown {
    Unknown::Text(value.to_string())
}

fn object(pairs: &[(&str, Unknown)]) -> Unknown {
    Unknown::Object(
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_string(), value.clone()))
            .collect(),
    )
}

/// A step that logs `start`, yields once, then logs `end` and answers its own
/// text. The value is the step's input, so an identity codec over `Schema.String`.
struct YieldingStep {
    calls: Arc<Mutex<Vec<String>>>,
    value: String,
    yielded: bool,
}

impl YieldingStep {
    fn new(input: Unknown, calls: Arc<Mutex<Vec<String>>>) -> Self {
        let value = match input {
            Unknown::Text(value) => value,
            other => panic!("the step reads a string, got {other:?}"),
        };
        Self {
            calls,
            value,
            yielded: false,
        }
    }
}

impl Future for YieldingStep {
    type Output = Result<Unknown, StepCause>;

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        if !this.yielded {
            this.yielded = true;
            this.calls
                .lock()
                .unwrap()
                .push(format!("start {}", this.value));
            // One turn later is enough to prove the parser waited rather than
            // running the siblings ahead of this answer.
            context.waker().wake_by_ref();
            return Poll::Pending;
        }
        this.calls
            .lock()
            .unwrap()
            .push(format!("end {}", this.value));
        Poll::Ready(Ok(Unknown::Text(this.value.clone())))
    }
}

/// The pin's `SchemaGetter.transformEffect` used for both directions.
fn yielding_text(calls: Arc<Mutex<Vec<String>>>) -> Schema<Unknown> {
    let decode_calls = Arc::clone(&calls);
    let encode_calls = Arc::clone(&calls);
    Schema::effect_codec(
        Schema::as_unknown(Schema::string()),
        move |input| Box::pin(YieldingStep::new(input, Arc::clone(&decode_calls))) as StepFuture,
        move |input| Box::pin(YieldingStep::new(input, Arc::clone(&encode_calls))) as StepFuture,
    )
}

fn options(errors: Errors) -> ParseOptions {
    ParseOptions {
        errors,
        ..ParseOptions::default()
    }
}

/// The four shapes the row drives, each over the one async codec, all erased to
/// `Unknown` so one loop can cover them.
fn product(shape: &str, item: &Schema<Unknown>) -> (Schema<Unknown>, Unknown) {
    let fields = || {
        object(&[("a", text("a")), ("b", text("b"))])
    };
    let positions = || Unknown::Array(vec![text("a"), text("b")]);
    match shape {
        "Struct" => (
            Schema::as_unknown(Schema::struct_shape(vec![
                Field::required("a", Schema::as_unknown(item.clone())),
                Field::required("b", Schema::as_unknown(item.clone())),
            ])),
            fields(),
        ),
        "Tuple" => (Schema::tuple_of(vec![item.clone(), item.clone()]), positions()),
        "Array" => (Schema::as_unknown(Schema::array(item.clone())), positions()),
        _ => (
            Schema::as_unknown(Schema::record(Schema::as_unknown(item.clone()))),
            fields(),
        ),
    }
}

/// A step that yields once, then refuses. The pin's
/// `SchemaGetter.transformOptionalEffect` over `Effect.fail`.
fn failing_text() -> Schema<Unknown> {
    Schema::effect_codec(
        Schema::as_unknown(Schema::string()),
        |_input| {
            Box::pin(async move {
                Err(StepCause::issue(SchemaError::TypeMismatch {
                    path: "$".to_string(),
                    expected: "a value the step accepts",
                    got: "string",
                }))
            }) as StepFuture
        },
        |input| Box::pin(std::future::ready(Ok(input))) as StepFuture,
    )
}

/// A struct tagged by a literal `kind`, which is how a union member is selected.
fn tagged_member(kind: &str, value: Schema<Unknown>) -> Schema<Unknown> {
    Schema::as_unknown(Schema::<BTreeMap<String, Unknown>>::struct_shape(vec![
        Field::required(
            "kind",
            Schema::as_unknown(Schema::<Unknown>::literal(text(kind))),
        ),
        Field::required("value", value),
    ]))
}

/// SchemaParser.test.ts:951 — when the selected member's step refuses, the parse
/// reports that refusal. The pin wraps it in an `AnyOf` issue; this port's decode
/// has one error channel, so the observable is the failure at the path the step
/// ran at, rather than a value produced as if the step had answered.
#[test]
fn an_asynchronous_failure_in_a_union_member_is_reported() {
    let schema = Schema::union_with_mode(
        vec![
            tagged_member("a", Schema::as_unknown(Schema::string())),
            tagged_member("b", failing_text()),
        ],
        "anyOf",
    );

    let exit = Runtime::run(decode_unknown_effect(
        &schema,
        object(&[("kind", text("b")), ("value", text("value"))]),
    ));
    let Exit::Failure(cause) = exit else {
        panic!("a refused step must not decode as if it answered, got {exit:?}");
    };
    let message = format!("{cause:?}");
    assert!(
        message.contains("path: \"$.value\""),
        "the refusal names the field the step ran at: {message}"
    );
}

/// SchemaParser.test.ts:975 — a candidate whose step refuses does not stop the
/// next candidate from taking the input unchanged.
#[test]
fn an_unchanged_union_candidate_still_resolves_after_a_sibling_fails() {
    let schema = Schema::union_with_mode(
        vec![failing_text(), Schema::as_unknown(Schema::string())],
        "anyOf",
    );

    let exit = Runtime::run(decode_unknown_effect(&schema, text("value")));
    let Exit::Success(Unknown::Text(value)) = exit else {
        panic!("the unchanged candidate takes the input, got {exit:?}");
    };
    assert_eq!(value, "value", "the value is unchanged");
}

/// SchemaParser.test.ts:45 — every product shape, both error modes, decode and
/// encode, in declaration order.
#[test]
fn asynchronous_children_run_sequentially_for_every_product_shape() {
    for errors in [Errors::First, Errors::All] {
        for shape in ["Struct", "Tuple", "Array", "Record"] {
            let calls = Arc::new(Mutex::new(Vec::new()));
            let item = yielding_text(Arc::clone(&calls));
            let (schema, input) = product(shape, &item);

            let exit = Runtime::run(decode_unknown_effect_with(
                &schema,
                input.clone(),
                options(errors),
            ));
            let Exit::Success(decoded) = exit else {
                panic!("{shape}/{errors:?}: decode failed: {exit:?}");
            };
            assert_eq!(
                calls.lock().unwrap().as_slice(),
                ["start a", "end a", "start b", "end b"],
                "{shape}/{errors:?}: decoding ran in declaration order"
            );

            calls.lock().unwrap().clear();
            let encoded =
                Runtime::run(encode_unknown_effect_with(&schema, &decoded, options(errors)));
            let Exit::Success(encoded) = encoded else {
                panic!("{shape}/{errors:?}: encode failed: {encoded:?}");
            };
            assert_eq!(
                encoded, input,
                "{shape}/{errors:?}: the identity step round-trips"
            );
            assert_eq!(
                calls.lock().unwrap().as_slice(),
                ["start a", "end a", "start b", "end b"],
                "{shape}/{errors:?}: encoding ran in declaration order"
            );
        }
    }
}

/// A failing step is an issue at its occurrence. `errors: "first"` stops at the
/// first one, `"all"` reports every sibling that failed.
#[test]
fn a_failing_step_reports_at_its_occurrence() {
    fn failing(calls: Arc<Mutex<Vec<String>>>, fails: &'static str) -> Schema<Unknown> {
        let decode_calls = Arc::clone(&calls);
        Schema::effect_codec(
            Schema::as_unknown(Schema::string()),
            move |input| {
                let value = match &input {
                    Unknown::Text(text) => text.clone(),
                    _ => panic!("expected text"),
                };
                decode_calls.lock().unwrap().push(value.clone());
                let answer = if value == fails {
                    Err(StepCause::issue(SchemaError::TypeMismatch {
                        path: "$".to_string(),
                        expected: "the accepted value",
                        got: "string",
                    }))
                } else {
                    Ok(input)
                };
                Box::pin(std::future::ready(answer)) as StepFuture
            },
            |input| Box::pin(std::future::ready(Ok(input))) as StepFuture,
        )
    }

    for errors in [Errors::First, Errors::All] {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let schema = Schema::struct_shape(vec![
            Field::required("a", failing(Arc::clone(&calls), "a")),
            Field::required("b", failing(Arc::clone(&calls), "b")),
        ]);
        let exit = Runtime::run(decode_unknown_effect_with(
            &schema,
            object(&[("a", text("a")), ("b", text("b"))]),
            options(errors),
        ));
        let Exit::Failure(cause) = exit else {
            panic!("{errors:?}: expected a refusal");
        };
        // The cause prints its reasons through `Debug`, so the path is what
        // separates the two occurrences.
        let message = format!("{cause:?}");
        assert!(message.contains("path: \"$.a\""), "{errors:?}: {message}");
        match errors {
            Errors::First => {
                assert!(
                    !message.contains("path: \"$.b\""),
                    "first stops early: {message}"
                );
                assert_eq!(calls.lock().unwrap().as_slice(), ["a"], "only a ran");
            }
            Errors::All => {
                assert!(
                    message.contains("path: \"$.b\""),
                    "all reports both: {message}"
                );
                assert_eq!(calls.lock().unwrap().as_slice(), ["a", "b"], "both ran");
            }
        }
    }
}

/// The effect adapter stays lazy: building the description runs no step.
#[test]
fn effect_descriptions_run_no_step_until_interpreted() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let schema = Schema::struct_shape(vec![Field::required(
        "a",
        Schema::as_unknown(yielding_text(Arc::clone(&calls))),
    )]);
    let effect = decode_unknown_effect(&schema, object(&[("a", text("a"))]));
    assert!(calls.lock().unwrap().is_empty(), "nothing ran at construction");
    let exit = Runtime::run(effect);
    assert!(matches!(exit, Exit::Success(_)), "{exit:?}");
    assert_eq!(calls.lock().unwrap().as_slice(), ["start a", "end a"]);
}

/// A sync adapter refuses a schema whose *child* carries a step, not only one
/// whose root does: the step is below the node the caller named.
///
/// The pin's `asExit` is `Effect.runSyncExit`, so the step starts and the parse
/// dies when it has to wait (`Effect.ts:9272`). "Did not wait" is the claim: the
/// step's start is logged, its end is not.
#[test]
fn a_nested_step_makes_the_sync_adapter_die() {
    let exit_calls = Arc::new(Mutex::new(Vec::new()));
    let exit_schema = Schema::struct_shape(vec![Field::required(
        "a",
        Schema::as_unknown(yielding_text(Arc::clone(&exit_calls))),
    )]);
    match decode_unknown_exit(&exit_schema, object(&[("a", text("a"))])) {
        Exit::Failure(cause) => assert!(
            cause.has_dies(),
            "the exit adapter dies rather than waiting for a step: {cause:?}"
        ),
        Exit::Success(value) => panic!("the exit adapter waited for a step: {value:?}"),
    }
    assert_eq!(
        exit_calls.lock().unwrap().as_slice(),
        ["start a"],
        "the step started, and the parse did not wait for its end"
    );

    let sync_calls = Arc::new(Mutex::new(Vec::new()));
    let sync_schema = Schema::struct_shape(vec![Field::required(
        "a",
        Schema::as_unknown(yielding_text(Arc::clone(&sync_calls))),
    )]);
    assert!(
        decode_unknown_sync(&sync_schema, object(&[("a", text("a"))])).is_err(),
        "the sync adapter refuses the same schema"
    );
    assert_eq!(
        sync_calls.lock().unwrap().as_slice(),
        ["start a"],
        "the sync adapter did not wait for the step either"
    );
}

/// A step is not run twice: the second phase decodes the answer the first phase
/// produced, so a codec whose answer differs from its input is not re-read.
#[test]
fn the_second_phase_does_not_replay_the_step() {
    let runs = Arc::new(AtomicUsize::new(0));
    let decode_runs = Arc::clone(&runs);
    let schema = Schema::effect_codec(
        Schema::as_unknown(Schema::string()),
        move |input| {
            decode_runs.fetch_add(1, Ordering::SeqCst);
            Box::pin(std::future::ready(Ok(input))) as StepFuture
        },
        |input| Box::pin(std::future::ready(Ok(input))) as StepFuture,
    );

    let exit = Runtime::run(decode_unknown_effect(&schema, text("value")));
    assert!(matches!(exit, Exit::Success(Unknown::Text(_))), "{exit:?}");
    assert_eq!(runs.load(Ordering::SeqCst), 1, "one run per parse");
}
