//! Slice 6: value formatting per schema kind.
//! Ports toFormatter.test.ts leaf cells by exact call.

use elm_effect_schema::{Schema, Unknown};

#[test]
fn formatter_never() {
    let format = Schema::never().formatter();
    assert_eq!(format(&Unknown::Number(1.0)), "never");
}

#[test]
fn formatter_any() {
    let format = Schema::any().formatter();
    assert_eq!(format(&Unknown::Number(1.0)), "1");
    assert_eq!(format(&Unknown::Text("a".to_string())), "\"a\"");
    assert_eq!(format(&Unknown::Bool(true)), "true");
    assert_eq!(format(&Unknown::Bool(false)), "false");
    assert_eq!(format(&Unknown::Null), "null");
}

#[test]
fn formatter_unknown() {
    let format = Schema::unknown().formatter();
    assert_eq!(format(&Unknown::Number(1.0)), "1");
    assert_eq!(format(&Unknown::Text("a".to_string())), "\"a\"");
    assert_eq!(format(&Unknown::Null), "null");
}

#[test]
fn formatter_leaves() {
    assert_eq!(
        Schema::as_unknown(Schema::string()).formatter()(&Unknown::Text("a".to_string())),
        "\"a\""
    );
    assert_eq!(
        Schema::as_unknown(Schema::number()).formatter()(&Unknown::Number(1.0)),
        "1"
    );
    assert_eq!(
        Schema::as_unknown(Schema::boolean()).formatter()(&Unknown::Bool(true)),
        "true"
    );
    let lit = Schema::literal(Unknown::Text("a".to_string())).formatter();
    assert_eq!(lit(&Unknown::Text("a".to_string())), "\"a\"");
}

#[test]
fn formatter_null() {
    let format = Schema::null().formatter();
    assert_eq!(format(&Unknown::Null), "null");
}
