//! The schema feature's public surface (`Schema`, `Unknown`, `SchemaError`)
//! is re-exported from `elm-effect`, and a caller resolves it through this
//! crate's name rather than `elm_effect_schema`. That assertion lives here
//! because this crate owns the re-export: putting it in `elm-effect-schema`
//! would make the schema crate dev-depend on `elm-effect`, which already
//! depends on `elm-effect-schema`, so the edge would run both ways.

use elm_effect::{Schema, SchemaError, Unknown};

#[test]
fn schema_types_are_reachable_from_elm_effect() {
    let schema = Schema::number();
    assert_eq!(
        schema.decode(Unknown::Number(1.0)).unwrap(),
        1.0,
        "Unknown decodes through the re-export"
    );
    // The error type is reachable too, and a bad value reaches it.
    let error: SchemaError = schema.decode_json(r#"{"oops"#).unwrap_err();
    assert!(!error.to_string().is_empty());
}

#[test]
fn reexported_schema_round_trips() {
    let schema = Schema::string();
    assert_eq!(schema.decode(Unknown::Text("hi".to_string())), Ok("hi".to_string()));
    assert_eq!(schema.encode(&"hi".to_string()), Unknown::Text("hi".to_string()));
}
