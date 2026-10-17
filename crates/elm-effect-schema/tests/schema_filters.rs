//! Filter results: what a `makeFilter` may answer, and what the chain around
//! several filters reports.
//!
//! Derives from Schema.test.ts:9199-9398, the `Schema.makeFilter` describe.

use elm_effect_schema::{FilterIssue, FilterOutcome, FilterSpec, Schema, SchemaError, Unknown};

fn text(value: &str) -> Unknown {
    Unknown::Text(value.to_string())
}

fn string() -> Schema<Unknown> {
    Schema::as_unknown(Schema::string())
}

/// A filter that always answers the same thing, with the annotations the pin
/// attaches.
fn answering(
    outcome: impl Fn() -> FilterOutcome + Send + Sync + 'static,
    annotation: Vec<(&str, &str)>,
) -> FilterSpec {
    FilterSpec::new(move |_| outcome()).annotated(
        annotation
            .into_iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect(),
    )
}

fn message(schema: &Schema<Unknown>, input: &str) -> String {
    schema.decode(text(input)).unwrap_err().to_string()
}

#[test]
fn filter_returning_undefined_passes() {
    // :9200 "returns undefined": a filter that answers nothing passes the value
    // through to the schema underneath.
    let schema = Schema::filtered(
        string(),
        vec![FilterSpec::new(|_| FilterOutcome::Pass)],
        false,
    );
    assert_eq!(schema.decode(text("a")), Ok(text("a")));
}

#[test]
fn filter_returning_true_passes() {
    // :9207 "returns true".
    let schema = Schema::filtered(
        string(),
        vec![FilterSpec::new(|_| FilterOutcome::from_bool(true))],
        false,
    );
    assert_eq!(schema.decode(text("a")), Ok(text("a")));
}

#[test]
fn filter_returning_false_names_the_filter() {
    // :9214 "returns false": without a message annotation the refusal is the
    // pin's `Expected <filter>`.
    let schema = Schema::filtered(
        string(),
        vec![FilterSpec::new(|_| FilterOutcome::from_bool(false))],
        false,
    );
    assert_eq!(message(&schema, "a"), "Expected <filter> at $");
}

#[test]
fn filter_returning_a_string_reports_it() {
    // :9224 "returns string": the message is the filter's own.
    let schema = Schema::filtered(
        string(),
        vec![FilterSpec::new(|_| {
            FilterOutcome::Fail(FilterIssue::new("error message"))
        })],
        false,
    );
    assert_eq!(message(&schema, "a"), "error message at $");
}

#[test]
fn filters_returning_issues_collect_in_order() {
    // :9235 "returns issue, abort: false": with every issue collected, both
    // filters are reported, the second one through its `message` annotation.
    let schema = Schema::filtered(
        string(),
        vec![
            answering(
                || FilterOutcome::Fail(FilterIssue::new("error message 1")),
                vec![("title", "filter title 1")],
            ),
            answering(
                || FilterOutcome::from_bool(false),
                vec![("title", "filter title 2"), ("message", "error message 2")],
            ),
        ],
        true,
    );
    assert_eq!(
        message(&schema, "a"),
        "error message 1 at $\nerror message 2 at $"
    );
    // Without the collecting mode the first failure is the whole answer.
    let first_only = Schema::filtered(
        string(),
        vec![
            answering(
                || FilterOutcome::Fail(FilterIssue::new("error message 1")),
                Vec::new(),
            ),
            answering(
                || FilterOutcome::Fail(FilterIssue::new("error message 2")),
                Vec::new(),
            ),
        ],
        false,
    );
    assert_eq!(message(&first_only, "a"), "error message 1 at $");
}

#[test]
fn an_aborting_filter_stops_the_chain() {
    // :9252 "returns issue, abort: true".
    let schema = Schema::filtered(
        string(),
        vec![
            answering(
                || FilterOutcome::Fail(FilterIssue::new("error message 1")),
                Vec::new(),
            )
            .aborting(),
            answering(
                || FilterOutcome::Fail(FilterIssue::new("error message 2")),
                Vec::new(),
            ),
        ],
        true,
    );
    assert_eq!(message(&schema, "a"), "error message 1 at $");
}

#[test]
fn a_filter_object_points_at_a_path() {
    // :9270 "returns object, abort: false": the pointed issue keeps its path
    // and the chain still reports the filter after it.
    let schema = Schema::filtered(
        string(),
        vec![
            answering(
                || FilterOutcome::Fail(FilterIssue::new("error message 1").at("a")),
                vec![("title", "filter title 1")],
            ),
            answering(
                || FilterOutcome::from_bool(false),
                vec![("title", "filter title 2"), ("message", "error message 2")],
            ),
        ],
        true,
    );
    assert_eq!(
        message(&schema, "a"),
        "error message 1 at $.a\nerror message 2 at $"
    );
}

#[test]
fn an_aborting_path_filter_stops_the_chain() {
    // :9289 "returns object, abort: true".
    let schema = Schema::filtered(
        string(),
        vec![
            answering(
                || FilterOutcome::Fail(FilterIssue::new("error message 1").at("a")),
                Vec::new(),
            )
            .aborting(),
            answering(
                || FilterOutcome::Fail(FilterIssue::new("error message 2")),
                Vec::new(),
            ),
        ],
        true,
    );
    assert_eq!(message(&schema, "a"), "error message 1 at $.a");
}

#[test]
fn a_filter_issue_carries_its_own_message() {
    // :9304 "issue: Issue": an issue value is a message, and the filter's
    // position is the path it was pointed at.
    let schema = Schema::filtered(
        string(),
        vec![FilterSpec::new(|_| {
            FilterOutcome::Fail(FilterIssue::new("custom issue").at("a"))
        })],
        false,
    );
    assert_eq!(message(&schema, "a"), "custom issue at $.a");
}

#[test]
fn an_empty_array_result_passes() {
    // :9323 "empty array is treated as success".
    let schema = Schema::filtered(
        string(),
        vec![FilterSpec::new(|_| FilterOutcome::Many(Vec::new()))],
        false,
    );
    assert_eq!(schema.decode(text("a")), Ok(text("a")));
}

#[test]
fn a_single_element_array_collapses() {
    // :9330 "single-element array collapses to the element".
    let schema = Schema::filtered(
        string(),
        vec![FilterSpec::new(|_| {
            FilterOutcome::Many(vec![FilterIssue::new("error message 1").at("a")])
        })],
        false,
    );
    assert_eq!(message(&schema, "a"), "error message 1 at $.a");
}

#[test]
fn a_multi_element_array_composites() {
    // :9343 "multi-element array groups into a Composite": one line per member,
    // each at its own path.
    let schema = Schema::filtered(
        string(),
        vec![FilterSpec::new(|_| {
            FilterOutcome::Many(vec![
                FilterIssue::new("error message 1").at("a"),
                FilterIssue::new("error message 2").at("b"),
            ])
        })],
        true,
    );
    assert_eq!(
        message(&schema, "a"),
        "error message 1 at $.a\nerror message 2 at $.b"
    );
}

#[test]
fn an_array_mixing_kinds_composites_in_order() {
    // :9361 "array mixing string, Issue, and { path, issue }": the order is the
    // array's, and only the pointed member carries a path.
    let schema = Schema::filtered(
        string(),
        vec![FilterSpec::new(|_| {
            FilterOutcome::Many(vec![
                FilterIssue::new("top-level message"),
                FilterIssue::new("direct issue"),
                FilterIssue::new("pointed message").at("a"),
            ])
        })],
        true,
    );
    assert_eq!(
        message(&schema, "a"),
        "top-level message at $\ndirect issue at $\npointed message at $.a"
    );
}

#[test]
fn a_filter_chain_decides_the_encode_side_too() {
    // The filters are the source schema's, so a value they refuse has no wire
    // form. The first failure is the answer here, as on decode.
    let schema = Schema::filtered(
        string(),
        vec![FilterSpec::new(|value| {
            FilterOutcome::from_bool(value == &text("ok"))
        })],
        true,
    );
    assert_eq!(schema.try_encode(&text("ok")), Ok(text("ok")));
    let refusal = schema.try_encode(&text("bad")).unwrap_err().to_string();
    assert_eq!(refusal, "Expected <filter> at $");
    // A composite is what a caller that may hold several sees: the type is the
    // same error, so a refusal through `decode` stays one value.
    assert!(matches!(
        schema.decode(text("bad")),
        Err(SchemaError::FilterFailed { path, message }) if path == "$" && message == "Expected <filter>"
    ));
}
