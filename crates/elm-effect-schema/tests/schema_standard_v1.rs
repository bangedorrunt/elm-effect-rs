//! `packages/effect/test/schema/toStandardSchemaV1.test.ts` V1 adapter witnesses.
//!
//! `toStandardSchemaV1` adapts a schema to `{ value } | { issues }` with
//! `message` + `path` issues. Async schemas, missing-dependency effects, and
//! `message`/`messageMissingKey`/`messageUnexpectedKey` annotation routing
//! need engine machinery this crate does not have, so those rows are excused;
//! every row below pins behavior the adapter plus existing decoders carry.

use elm_effect_schema::{
    PathSegment, Schema, StandardSchemaV1, Unknown, V1Errors, to_standard_json_schema_v1,
    to_standard_schema_v1,
};

fn assert_value<T: PartialEq + std::fmt::Debug>(result: elm_effect_schema::V1Result<T>, expected: T, _row: &str) {
    match result {
        elm_effect_schema::V1Result::Value(value) => assert_eq!(value, expected),
        elm_effect_schema::V1Result::Issues(issues) => panic!("expected value, got {issues:?}"),
    }
}

fn assert_issues<T: std::fmt::Debug>(
    result: elm_effect_schema::V1Result<T>,
    expected: Vec<(&str, Vec<PathSegment>)>,
    _row: &str,
) {
    match result {
        elm_effect_schema::V1Result::Value(value) => panic!("expected issues, got {value:?}"),
        elm_effect_schema::V1Result::Issues(issues) => {
            let got: Vec<(String, Vec<PathSegment>)> = issues
                .into_iter()
                .map(|issue| (issue.message, issue.path))
                .collect();
            let want: Vec<(String, Vec<PathSegment>)> = expected
                .into_iter()
                .map(|(message, path)| (message.to_owned(), path))
                .collect();
            assert_eq!(got, want);
        }
    }
}

fn non_empty() -> Schema<Unknown> {
    use elm_effect_schema::{FilterOutcome, FilterSpec};
    Schema::filtered(
        Schema::as_unknown(Schema::string()),
        vec![FilterSpec::new(|value| {
            FilterOutcome::from_bool(!matches!(value, Unknown::Text(text) if text.is_empty()))
        })
        .annotated(vec![("message".to_owned(), "a value with a length of at least 1".to_owned())])],
        false,
    )
}

#[test]
fn returns_a_standard_schema_v1_schema() {
    // toStandardSchemaV1.test.ts:105 — the adapter validates like the schema.
    let standard = to_standard_schema_v1(Schema::finite_from_string());
    assert_value(standard.validate(Unknown::Text("1.5".into())), 1.5, ":105");
}

#[test]
fn supports_both_standards() {
    // toStandardSchemaV1.test.ts:111 — composed with the JSON Schema wrapper.
    let wrapped = to_standard_json_schema_v1(Schema::string());
    let both = to_standard_schema_v1(wrapped);
    assert_value(both.validate(Unknown::Text("a".into())), "a".to_owned(), ":111");
    assert_eq!(
        both.input(elm_effect_schema::StandardJsonSchemaTarget::Draft2020_12),
        serde_json::json!({"type": "string"})
    );
}

#[test]
fn sync_decoding_reports_message_and_path() {
    // toStandardSchemaV1.test.ts:117
    let standard = to_standard_schema_v1(non_empty());
    assert_value(standard.validate(Unknown::Text("a".into())), Unknown::Text("a".into()), ":117");
    assert_issues(
        standard.validate(Unknown::Null),
        vec![("Expected string", vec![])],
        ":117",
    );
    assert_issues(
        standard.validate(Unknown::Text(String::new())),
        vec![("Expected a value with a length of at least 1", vec![])],
        ":117",
    );
}

#[test]
fn by_default_returns_all_issues() {
    // toStandardSchemaV1.test.ts:198
    let schema = Schema::struct_([("a", non_empty()), ("b", non_empty())]);
    let standard = to_standard_schema_v1(schema);
    match standard.validate(Unknown::Object(
        [("a".to_owned(), Unknown::Text("a".into())), ("b".to_owned(), Unknown::Text("b".into()))]
            .into_iter()
            .collect(),
    )) {
        elm_effect_schema::V1Result::Value(value) => {
            assert_eq!(value.get("a"), Some(&Unknown::Text("a".into())));
            assert_eq!(value.get("b"), Some(&Unknown::Text("b".into())));
        }
        elm_effect_schema::V1Result::Issues(issues) => panic!("expected value, got {issues:?}"),
    }
    // A success case above would need a value comparison; failures pin paths.
    assert_issues(
        standard.validate(Unknown::Null),
        vec![("Expected object", vec![])],
        ":198-null",
    );
    assert_issues(
        standard.validate(Unknown::Object(
            [("a".to_owned(), Unknown::Text(String::new())), ("b".to_owned(), Unknown::Text(String::new()))]
                .into_iter()
                .collect(),
        )),
        vec![
            ("Expected a value with a length of at least 1", vec![PathSegment::Key("a".into())]),
            ("Expected a value with a length of at least 1", vec![PathSegment::Key("b".into())]),
        ],
        ":198-both",
    );
}

#[test]
fn with_first_errors_returns_only_the_first_issue() {
    // toStandardSchemaV1.test.ts:235
    let schema = Schema::struct_([("a", non_empty()), ("b", non_empty())]);
    let standard = to_standard_schema_v1(schema).with_errors(V1Errors::First);
    assert_issues(
        standard.validate(Unknown::Object(
            [("a".to_owned(), Unknown::Text(String::new())), ("b".to_owned(), Unknown::Text(String::new()))]
                .into_iter()
                .collect(),
        )),
        vec![("Expected a value with a length of at least 1", vec![PathSegment::Key("a".into())])],
        ":235",
    );
}

#[test]
fn leaf_hook_string_reports_the_type_message() {
    // toStandardSchemaV1.test.ts: leafHook on String — the default hook is
    // output-identity, so the message assertions are the row's substance.
    let standard = to_standard_schema_v1(Schema::as_unknown(Schema::string()));
    assert_issues(
        standard.validate(Unknown::Null),
        vec![("Expected string", vec![])],
        ":leaf-string",
    );
}

#[test]
fn leaf_hook_non_empty_string_reports_the_check_message() {
    // toStandardSchemaV1.test.ts: leafHook on NonEmptyString.
    let standard = to_standard_schema_v1(non_empty());
    assert_issues(
        standard.validate(Unknown::Text(String::new())),
        vec![("Expected a value with a length of at least 1", vec![])],
        ":leaf-nonempty",
    );
}
