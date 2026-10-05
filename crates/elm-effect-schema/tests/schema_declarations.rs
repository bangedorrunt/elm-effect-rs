//! Slice 2: declaration-table cells (literals, union, builtins).
//! Ports packages/effect/test/schema/Schema.test.ts cells by exact call.

use elm_effect_schema::{Schema, Unknown};

fn lit(text: &str) -> Schema<Unknown> {
    Schema::literal(Unknown::Text(text.to_string()))
}

#[test]
fn cell_literal_a() {
    let schema = lit("a");
    assert_eq!(schema.decode(Unknown::Text("a".to_string())), Ok(Unknown::Text("a".to_string())));
    assert!(schema.decode(Unknown::Null).is_err());
    assert!(schema.decode(Unknown::Number(1.0)).is_err());
    assert_eq!(schema.encode(&Unknown::Text("a".to_string())), Unknown::Text("a".to_string()));
}

#[test]
fn cell_literal_1() {
    let schema = Schema::literal(Unknown::Number(1.0));
    assert_eq!(schema.decode(Unknown::Number(1.0)), Ok(Unknown::Number(1.0)));
    assert!(schema.decode(Unknown::Null).is_err());
    assert!(schema.decode(Unknown::Text("1".to_string())).is_err());
}

#[test]
fn cell_literal_union_red_green_blue() {
    let schema = Schema::union(
        Schema::union(lit("red"), lit("green")),
        lit("blue"),
    );
    for color in ["red", "green", "blue"] {
        assert_eq!(
            schema.decode(Unknown::Text(color.to_string())),
            Ok(Unknown::Text(color.to_string()))
        );
    }
    assert!(schema.decode(Unknown::Text("yellow".to_string())).is_err());
    assert!(schema.decode(Unknown::Null).is_err());
}

#[test]
fn cell_string() {
    let schema = Schema::as_unknown(Schema::string());
    assert_eq!(
        schema.decode(Unknown::Text("a".to_string())),
        Ok(Unknown::Text("a".to_string()))
    );
    assert!(schema.decode(Unknown::Null).is_err());
}

#[test]
fn cell_number() {
    let schema = Schema::as_unknown(Schema::number());
    assert_eq!(schema.decode(Unknown::Number(1.0)), Ok(Unknown::Number(1.0)));
    assert!(schema.decode(Unknown::Null).is_err());
    assert!(schema.decode(Unknown::Text("1".to_string())).is_err());
}

#[test]
fn cell_boolean() {
    let schema = Schema::as_unknown(Schema::boolean());
    assert_eq!(schema.decode(Unknown::Bool(true)), Ok(Unknown::Bool(true)));
    assert!(schema.decode(Unknown::Null).is_err());
    assert!(schema.decode(Unknown::Number(1.0)).is_err());
}
