//! The pin's bounded-concurrency rows for the effect parser.
//!
//! Each test drives a parse on its own thread and observes the step log from
//! this one, which is the pin's `Effect.forkChild` + `Deferred` dance: a gate
//! is opened from outside the fiber, and the fiber parks until a step's waker
//! runs. `SchemaParser.test.ts:84` (bounded, both axes), `:116` (unbounded),
//! `:137` (interruption), `:180` (completion order), `:221` (record key
//! collision) and `:247` (nested limits) are the rows.

#![cfg(all(feature = "schema", feature = "tokio"))]

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};
use std::time::{Duration, Instant};

use elm_effect::{Exit, Runtime, decode_unknown_effect_with, encode_unknown_effect_with};
use elm_effect_schema::transform::{Concurrency, Errors, ParseOptions};
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

/// A one-shot gate: `open` releases every waiter. The pin's `Deferred`.
#[derive(Clone, Default)]
struct Gate(Arc<Mutex<GateState>>);

#[derive(Default)]
struct GateState {
    open: bool,
    waiters: Vec<Waker>,
}

impl Gate {
    fn open(&self) {
        let mut state = self.0.lock().unwrap();
        state.open = true;
        for waiter in state.waiters.drain(..) {
            waiter.wake();
        }
    }

    fn is_open(&self) -> bool {
        self.0.lock().unwrap().open
    }

    fn wait(&self) -> Wait {
        Wait(self.clone())
    }
}

struct Wait(Gate);

impl Future for Wait {
    type Output = ();

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<()> {
        let mut state = self.0 .0.lock().unwrap();
        if state.open {
            return Poll::Ready(());
        }
        state.waiters.push(context.waker().clone());
        Poll::Pending
    }
}

/// Wait for a state the parse reaches, with a ceiling so a broken scheduler
/// fails the test instead of hanging the suite.
fn wait_until(what: &str, predicate: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !predicate() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn options(concurrency: Concurrency, errors: Errors) -> ParseOptions {
    ParseOptions {
        concurrency,
        errors,
        ..ParseOptions::default()
    }
}

/// A codec whose decode runs `decode`, with an identity encode.
fn codec(
    decode: impl Fn(Unknown) -> StepFuture + Send + Sync + 'static,
) -> Schema<Unknown> {
    Schema::effect_codec(
        Schema::as_unknown(Schema::string()),
        decode,
        |input| Box::pin(std::future::ready(Ok(input))) as StepFuture,
    )
}

fn letter(input: &Unknown) -> (String, usize) {
    let value = match input {
        Unknown::Text(value) => value.clone(),
        other => panic!("the step reads a string, got {other:?}"),
    };
    let index = (value.as_bytes()[0] - b'a') as usize;
    (value, index)
}

/// A step that opens its own start gate, then waits for its release gate.
///
/// Both directions share it, so the encode axis is scheduled the way the decode
/// axis is and the row's `for (decode, encode)` loop is really both axes.
fn gated_step(
    started: &[Gate],
    releases: &[Gate],
) -> impl Fn(Unknown) -> StepFuture + Send + Sync + 'static {
    let started: Vec<Gate> = started.to_vec();
    let releases: Vec<Gate> = releases.to_vec();
    move |input| {
        let (value, index) = letter(&input);
        let started = started.clone();
        let releases = releases.clone();
        Box::pin(async move {
            started[index].open();
            releases[index].wait().await;
            Ok(text(&value))
        }) as StepFuture
    }
}

/// A codec gated in both directions.
fn gated(started: &[Gate], releases: &[Gate]) -> Schema<Unknown> {
    Schema::effect_codec(
        Schema::as_unknown(Schema::string()),
        gated_step(started, releases),
        gated_step(started, releases),
    )
}

/// Three positions named `a`, `b`, `c` in the shape the row asks for.
fn three(shape: &str, item: &Schema<Unknown>) -> (Schema<Unknown>, Unknown) {
    let names = ["a", "b", "c"];
    let fields = || {
        object(&names.map(|name| (name, text(name))))
    };
    let positions = || Unknown::Array(names.iter().map(|name| text(name)).collect());
    match shape {
        "Struct" => (
            Schema::as_unknown(Schema::struct_shape(
                names
                    .iter()
                    .map(|name| Field::required(name, Schema::as_unknown(item.clone())))
                    .collect(),
            )),
            fields(),
        ),
        "Tuple" => (
            Schema::tuple_of(vec![item.clone(), item.clone(), item.clone()]),
            positions(),
        ),
        "Array" => (Schema::as_unknown(Schema::array(item.clone())), positions()),
        _ => (
            Schema::as_unknown(Schema::record(Schema::as_unknown(item.clone()))),
            fields(),
        ),
    }
}

/// SchemaParser.test.ts:84 — with `concurrency: 2`, the third child does not
/// start until a slot frees, on both axes and for every product shape.
#[test]
fn bounded_concurrency_holds_a_slot_back_for_each_shape() {
    for shape in ["Struct", "Tuple", "Array", "Record"] {
        for encode in [false, true] {
            let started: Vec<Gate> = (0..3).map(|_| Gate::default()).collect();
            let releases: Vec<Gate> = (0..3).map(|_| Gate::default()).collect();
            let (schema, input) = three(shape, &gated(&started, &releases));
            let run_schema = schema.clone();
            let run_input = input.clone();
            // The encode axis needs the decoded value, which is the input for an
            // identity codec; decoding first would consume the gates, so the two
            // axes get their own schema and gates.
            let handle = std::thread::spawn(move || {
                let effect = if encode {
                    encode_unknown_effect_with(
                        &run_schema,
                        &run_input,
                        options(Concurrency::Bounded(2), Errors::First),
                    )
                } else {
                    decode_unknown_effect_with(
                        &run_schema,
                        run_input,
                        options(Concurrency::Bounded(2), Errors::First),
                    )
                };
                Runtime::run(effect)
            });

            let label = format!("{shape} encode={encode}");
            wait_until(&format!("{label}: the first two children to start"), || {
                started[0].is_open() && started[1].is_open()
            });
            assert!(
                !started[2].is_open(),
                "{label}: the third child ran with two slots taken"
            );

            releases[0].open();
            wait_until(&format!("{label}: the third child after a slot freed"), || {
                started[2].is_open()
            });
            releases[1].open();
            releases[2].open();

            let exit = handle.join().expect("the parse thread finishes");
            assert!(matches!(exit, Exit::Success(_)), "{shape}: {exit:?}");
        }
    }
}

/// SchemaParser.test.ts:116 — `"unbounded"` starts every child at once.
#[test]
fn unbounded_concurrency_starts_every_child() {
    let started: Vec<Gate> = (0..3).map(|_| Gate::default()).collect();
    let releases: Vec<Gate> = (0..3).map(|_| Gate::default()).collect();
    let (schema, input) = three("Array", &gated(&started, &releases));
    let handle = std::thread::spawn(move || {
        Runtime::run(decode_unknown_effect_with(
            &schema,
            input,
            options(Concurrency::Unbounded, Errors::First),
        ))
    });

    wait_until("all three children to start", || {
        started.iter().all(Gate::is_open)
    });
    releases.iter().for_each(Gate::open);
    assert!(matches!(
        handle.join().expect("the parse thread finishes"),
        Exit::Success(_)
    ));
}

/// A step that never answers, recording when it is dropped.
struct Never {
    started: Gate,
    interrupted: Arc<Mutex<bool>>,
}

impl Future for Never {
    type Output = Result<Unknown, SchemaError>;

    fn poll(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Self::Output> {
        self.started.open();
        Poll::Pending
    }
}

impl Drop for Never {
    fn drop(&mut self) {
        *self.interrupted.lock().unwrap() = true;
    }
}

/// SchemaParser.test.ts:137 — the first failure to complete wins and the child
/// still pending is interrupted. The port reports the single issue as itself
/// rather than as a one-member `Composite` (its decode has one error channel),
/// so the assertion is the path and the interruption.
#[test]
fn the_first_failure_interrupts_the_pending_child() {
    let first_started = Gate::default();
    let interrupted = Arc::new(Mutex::new(false));

    let first = codec({
        let started = first_started.clone();
        let interrupted = Arc::clone(&interrupted);
        move |_input| {
            Box::pin(Never {
                started: started.clone(),
                interrupted: Arc::clone(&interrupted),
            }) as StepFuture
        }
    });
    let second = codec({
        let started = first_started.clone();
        move |_input| {
            let started = started.clone();
            Box::pin(async move {
                started.wait().await;
                Err(SchemaError::TypeMismatch {
                    path: "$".to_string(),
                    expected: "the second child",
                    got: "string",
                })
            }) as StepFuture
        }
    });
    let schema = Schema::tuple_of(vec![first, second]);

    let exit = Runtime::run(decode_unknown_effect_with(
        &schema,
        Unknown::Array(vec![text("a"), text("b")]),
        options(Concurrency::Bounded(2), Errors::First),
    ));

    let Exit::Failure(cause) = exit else {
        panic!("expected a refusal, got {exit:?}");
    };
    assert!(
        format!("{cause:?}").contains("path: \"$[1]\""),
        "the failure names the second position: {cause:?}"
    );
    assert!(
        *interrupted.lock().unwrap(),
        "the pending child was interrupted"
    );
}

/// SchemaParser.test.ts:180 — `errors: "all"` collects in completion order, not
/// declaration order.
#[test]
fn all_errors_accumulate_in_completion_order() {
    let release_first = Gate::default();
    let second_done = Gate::default();

    let failing = |wait: Option<Gate>, signal: Option<Gate>| {
        codec(move |_input| {
            let wait = wait.clone();
            let signal = signal.clone();
            Box::pin(async move {
                if let Some(wait) = wait {
                    wait.wait().await;
                }
                if let Some(signal) = signal {
                    signal.open();
                }
                Err(SchemaError::TypeMismatch {
                    path: "$".to_string(),
                    expected: "a valid value",
                    got: "string",
                })
            }) as StepFuture
        })
    };
    let schema = Schema::tuple_of(vec![
        failing(Some(release_first.clone()), None),
        failing(None, Some(second_done.clone())),
    ]);

    let handle = std::thread::spawn(move || {
        Runtime::run(decode_unknown_effect_with(
            &schema,
            Unknown::Array(vec![text("a"), text("b")]),
            options(Concurrency::Bounded(2), Errors::All),
        ))
    });
    wait_until("the second child to complete", || second_done.is_open());
    release_first.open();

    let exit = handle.join().expect("the parse thread finishes");
    let Exit::Failure(cause) = exit else {
        panic!("expected a refusal, got {exit:?}");
    };
    let message = format!("{cause:?}");
    let second = message.find("path: \"$[1]\"").expect("the second issue");
    let first = message.find("path: \"$[0]\"").expect("the first issue");
    assert!(
        second < first,
        "completion order puts the second child's issue first: {message}"
    );
}

/// SchemaParser.test.ts:221 — two record keys that decode onto one name are
/// decided by completion order: the last to finish keeps the name.
#[test]
fn a_record_key_collision_is_decided_by_completion_order() {
    let release_first = Gate::default();
    let second_done = Gate::default();

    // Both wire keys decode onto the same name, which is what the pin's
    // snakeToCamel key mapping does for `a_b` and `aB`.
    let key = Schema::convert(
        Schema::<String>::string(),
        |_| Ok("key".to_string()),
        |_| "key".to_string(),
    );
    let value = codec({
        let release_first = release_first.clone();
        let second_done = second_done.clone();
        move |input| {
            let value = match &input {
                Unknown::Text(value) => value.clone(),
                other => panic!("the step reads a string, got {other:?}"),
            };
            let release_first = release_first.clone();
            let second_done = second_done.clone();
            Box::pin(async move {
                if value == "first" {
                    release_first.wait().await;
                } else {
                    second_done.open();
                }
                Ok(Unknown::Text(value))
            }) as StepFuture
        }
    });
    let schema = Schema::as_unknown(Schema::<BTreeMap<String, Unknown>>::record2(
        Schema::as_unknown(key),
        value,
    ));

    let handle = std::thread::spawn(move || {
        Runtime::run(decode_unknown_effect_with(
            &schema,
            object(&[("a_b", text("first")), ("aB", text("second"))]),
            options(Concurrency::Bounded(2), Errors::First),
        ))
    });
    wait_until("the second key to complete", || second_done.is_open());
    release_first.open();

    let exit = handle.join().expect("the parse thread finishes");
    let Exit::Success(Unknown::Object(values)) = exit else {
        panic!("expected a value, got {exit:?}");
    };
    assert_eq!(
        values.get("key"),
        Some(&text("first")),
        "the last completion keeps the colliding name"
    );
}

/// SchemaParser.test.ts:247 — the limit applies to each product independently,
/// so two arrays of two run all four children under `concurrency: 2`.
#[test]
fn the_limit_applies_independently_to_nested_products() {
    let started: Vec<Gate> = (0..4).map(|_| Gate::default()).collect();
    let release = Gate::default();
    let count = Arc::new(Mutex::new(0usize));

    let item = codec({
        let started = started.clone();
        let release = release.clone();
        let count = Arc::clone(&count);
        move |input| {
            let (value, _) = letter(&input);
            let index = match value.as_str() {
                "a" => 0,
                "b" => 1,
                "c" => 2,
                _ => 3,
            };
            let started = started.clone();
            let release = release.clone();
            let count = Arc::clone(&count);
            Box::pin(async move {
                *count.lock().unwrap() += 1;
                started[index].open();
                release.wait().await;
                Ok(Unknown::Text(value))
            }) as StepFuture
        }
    });
    let inner = Schema::array(item);
    let schema = Schema::as_unknown(Schema::array(inner));
    let input = Unknown::Array(vec![
        Unknown::Array(vec![text("a"), text("b")]),
        Unknown::Array(vec![text("c"), text("d")]),
    ]);

    let handle = std::thread::spawn(move || {
        Runtime::run(decode_unknown_effect_with(
            &schema,
            input,
            options(Concurrency::Bounded(2), Errors::First),
        ))
    });
    wait_until("every nested child to start", || {
        *count.lock().unwrap() == 4
    });
    assert_eq!(
        *count.lock().unwrap(),
        4,
        "each inner array got its own two slots"
    );
    release.open();
    assert!(matches!(
        handle.join().expect("the parse thread finishes"),
        Exit::Success(_)
    ));
}
