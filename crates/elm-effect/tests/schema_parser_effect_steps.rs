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
use elm_effect_schema::{Field, Schema, SchemaError, StepFuture, Unknown};

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
    type Output = Result<Unknown, SchemaError>;

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
                    Err(SchemaError::TypeMismatch {
                        path: "$".to_string(),
                        expected: "the accepted value",
                        got: "string",
                    })
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
#[test]
fn a_nested_step_makes_the_sync_adapter_die() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let schema = Schema::struct_shape(vec![Field::required(
        "a",
        Schema::as_unknown(yielding_text(Arc::clone(&calls))),
    )]);
    assert!(
        matches!(
            decode_unknown_exit(&schema, object(&[("a", text("a"))])),
            Exit::Failure(_)
        ),
        "the exit adapter dies rather than running a step synchronously"
    );
    assert!(
        decode_unknown_sync(&schema, object(&[("a", text("a"))])).is_err(),
        "the sync adapter refuses the same schema"
    );
    assert!(calls.lock().unwrap().is_empty(), "no step ran");
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
