//! `Schema.test.ts` NonEmptyArray: the element schema it exposes and the
//! empty-array refusal on both axes.
//!
//!     cargo test -p elm-effect-schema --test schema_non_empty_array
//!
//! The pin builds this shape as a tuple with one required head and a rest, so
//! an empty array is a missing key at position 0 rather than a length check.
//! This port keeps that wording: the refusal lands at `$[0]` and says an
//! element is missing.

use elm_effect_schema::{Schema, SchemaError, Unknown};

fn text(value: &str) -> Unknown {
    Unknown::Text(value.to_string())
}

fn array(items: Vec<Unknown>) -> Unknown {
    Unknown::Array(items)
}

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| item.to_string()).collect()
}

/// The `(path, expected)` pair a refusal carries.
fn refusal(error: &SchemaError) -> (String, String) {
    match error {
        SchemaError::TypeMismatch { path, expected, .. } => (path.clone(), (*expected).to_string()),
        SchemaError::TypeMismatchDynamic { path, expected, .. } => (path.clone(), expected.clone()),
        other => panic!("not a shape refusal: {other}"),
    }
}

/// Schema.test.ts:1245 — "should expose the element schema via .value".
#[test]
fn non_empty_array_exposes_the_element_schema() {
    let schema = Schema::non_empty_array(Schema::string());
    let element = schema
        .element()
        .expect("a NonEmptyArray keeps its element schema");
    assert_eq!(element.to_code(), "Schema.String");
    assert_eq!(element.decode(text("a")), Ok(text("a")));
    assert!(element.decode(Unknown::Number(1.0)).is_err());
}

/// Schema.test.ts:1250 — `readonly string[]`. One element is enough, zero is
/// not, and a refusal names the position it happened at.
#[test]
fn non_empty_array_needs_at_least_one_element() {
    let schema = Schema::non_empty_array(Schema::string());

    // `make` is decoding here: the port has no separate construction rule for
    // this shape.
    assert_eq!(
        schema.construct(array(vec![text("a")])).unwrap(),
        strings(&["a"])
    );
    assert_eq!(
        schema.construct(array(vec![text("a"), text("b")])).unwrap(),
        strings(&["a", "b"])
    );

    assert_eq!(
        schema.decode(array(vec![text("a")])).unwrap(),
        strings(&["a"])
    );
    assert_eq!(
        schema.decode(array(vec![text("a"), text("b")])).unwrap(),
        strings(&["a", "b"])
    );
    assert_eq!(
        refusal(&schema.decode(array(vec![])).unwrap_err()),
        ("$[0]".to_string(), "an element".to_string())
    );
    assert_eq!(
        refusal(
            &schema
                .decode(array(vec![text("a"), Unknown::Number(1.0)]))
                .unwrap_err()
        ),
        ("$[1]".to_string(), "string".to_string())
    );

    assert_eq!(
        schema.try_encode(&strings(&["a"])).unwrap(),
        array(vec![text("a")])
    );
    assert_eq!(
        schema.try_encode(&strings(&["a", "b"])).unwrap(),
        array(vec![text("a"), text("b")])
    );
    // The emptiness rule holds on the way out too, so a caller cannot encode a
    // value the shape does not describe.
    assert_eq!(
        refusal(&schema.try_encode(&strings(&[])).unwrap_err()),
        ("$[0]".to_string(), "an element".to_string())
    );
}
