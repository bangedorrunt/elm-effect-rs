//! `Schema.test.ts` Result box: the tag object each side travels as, and the
//! two schemas it exposes.
//!
//!     cargo test -p elm-effect-schema --test schema_result
//!
//! As with `Schema.Option`, the pin's encoded type is a JS `Result` instance
//! while this port's wire is JSON, so the box travels as the tag object the
//! pin's codec links to: `{ "_tag": "Success", success }` or
//! `{ "_tag": "Failure", failure }`. The decoded value is Rust's
//! `Result<A, E>`, so the pin's `Result.succeed`/`Result.fail` are `Ok`/`Err`.

use elm_effect_schema::{Schema, SchemaError, Unknown};
use std::collections::BTreeMap;

fn text(value: &str) -> Unknown {
    Unknown::Text(value.to_string())
}

fn obj(pairs: Vec<(&str, Unknown)>) -> Unknown {
    Unknown::Object(
        pairs
            .into_iter()
            .map(|(key, value)| (key.to_string(), value))
            .collect::<BTreeMap<_, _>>(),
    )
}

/// The `(path, expected)` pair a refusal carries.
fn refusal(error: &SchemaError) -> (String, String) {
    match error {
        SchemaError::TypeMismatch { path, expected, .. } => (path.clone(), (*expected).to_string()),
        SchemaError::TypeMismatchDynamic { path, expected, .. } => (path.clone(), expected.clone()),
        other => panic!("not a shape refusal: {other}"),
    }
}

/// Schema.test.ts:3268 — "should expose the values". Both sides are readable,
/// and an annotation keeps them readable.
#[test]
fn result_exposes_the_success_and_failure_schemas() {
    let schema = Schema::result(Schema::string(), Schema::number());
    assert_eq!(
        schema
            .success()
            .expect("a Result keeps its success schema")
            .to_code(),
        "Schema.String"
    );
    assert_eq!(
        schema
            .failure()
            .expect("a Result keeps its failure schema")
            .to_code(),
        "Schema.Number"
    );

    let annotated = Schema::annotated_with(schema, vec![("title", "a result")]);
    assert_eq!(
        annotated
            .success()
            .expect("an annotation keeps the success schema")
            .to_code(),
        "Schema.String"
    );
    assert_eq!(
        annotated
            .failure()
            .expect("an annotation keeps the failure schema")
            .to_code(),
        "Schema.Number"
    );
}

/// Schema.test.ts:3276 — `Result(FiniteFromString, FiniteFromString)`. Each
/// tag reads and writes its own side, and a refusal lands on that side.
#[test]
fn result_decodes_and_encodes_the_tag_object() {
    let schema = Schema::result(Schema::finite_from_string(), Schema::finite_from_string());

    assert_eq!(
        schema
            .decode(obj(vec![("_tag", text("Success")), ("success", text("1"))]))
            .unwrap(),
        Ok(1.0)
    );
    assert_eq!(
        schema
            .decode(obj(vec![("_tag", text("Failure")), ("failure", text("2"))]))
            .unwrap(),
        Err(2.0)
    );

    // Not a Result at all, in either tagless spelling.
    assert_eq!(
        refusal(&schema.decode(Unknown::Null).unwrap_err()),
        ("$".to_string(), "Result".to_string())
    );
    assert_eq!(
        refusal(&schema.decode(obj(vec![])).unwrap_err()).0,
        "$".to_string()
    );

    // The side's own refusal keeps the side's path.
    assert_eq!(
        refusal(
            &schema
                .decode(obj(vec![("_tag", text("Success")), ("success", text("a"))]))
                .unwrap_err()
        )
        .0,
        "$.success"
    );
    assert_eq!(
        refusal(
            &schema
                .decode(obj(vec![("_tag", text("Failure")), ("failure", text("b"))]))
                .unwrap_err()
        )
        .0,
        "$.failure"
    );
    // A tag with no value is not a shape the box has.
    assert_eq!(
        refusal(
            &schema
                .decode(obj(vec![("_tag", text("Success"))]))
                .unwrap_err()
        ),
        ("$.success".to_string(), "required field".to_string())
    );

    assert_eq!(
        schema.try_encode(&Ok(1.0)).unwrap(),
        obj(vec![("_tag", text("Success")), ("success", text("1"))])
    );
    assert_eq!(
        schema.try_encode(&Err(2.0)).unwrap(),
        obj(vec![("_tag", text("Failure")), ("failure", text("2"))])
    );
}
