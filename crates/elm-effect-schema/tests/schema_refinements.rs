//! The named refinements as schemas: `Char`, `Int`, `Natural`, the four case
//! schemas and `isUUID`.
//!
//! Derives from Schema.test.ts:10108-10361.

use elm_effect_schema::{Schema, Unknown};

fn text(value: &str) -> Unknown {
    Unknown::Text(value.to_string())
}

fn message(schema: &Schema<String>, input: Unknown) -> String {
    schema.decode(input).unwrap_err().to_string()
}

#[test]
fn char_admits_exactly_one_character() {
    // :10108 "Char": one character, both directions.
    let schema = Schema::char();
    assert_eq!(schema.decode(text("a")), Ok("a".to_string()));
    assert_eq!(
        message(&schema, text("ab")),
        "expected a value with a length of 1 at $, got string"
    );
    assert_eq!(schema.encode(&"a".to_string()), text("a"));
    assert_eq!(
        schema
            .try_encode(&"ab".to_string())
            .unwrap_err()
            .to_string(),
        "expected a value with a length of 1 at $, got string"
    );
}

#[test]
fn int_is_an_integer_and_refuses_the_non_integers() {
    // :10131 "Int": a fraction, `NaN` and both infinities are all refused, on
    // both sides.
    let schema = Schema::int();
    assert_eq!(schema.decode(Unknown::Number(1.0)), Ok(1.0));
    for refused in [1.1, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert_eq!(
            schema
                .decode(Unknown::Number(refused))
                .unwrap_err()
                .to_string(),
            "expected an integer at $, got number",
            "{refused}"
        );
    }
    assert_eq!(schema.encode(&1.0), Unknown::Number(1.0));
    assert_eq!(
        schema.try_encode(&1.1).unwrap_err().to_string(),
        "expected an integer at $, got number"
    );
    // The wire type is still a number, so a string is refused by that, not by
    // the refinement.
    assert!(
        schema
            .decode(text("1"))
            .unwrap_err()
            .to_string()
            .contains("got string")
    );
}

#[test]
fn natural_is_a_non_negative_integer() {
    // :10166 "Natural": 0 and 1 pass; a negative integer names the bound and a
    // fraction names the integer rule.
    let schema = Schema::natural();
    assert_eq!(schema.decode(Unknown::Number(0.0)), Ok(0.0));
    assert_eq!(schema.decode(Unknown::Number(1.0)), Ok(1.0));
    assert_eq!(
        schema
            .decode(Unknown::Number(-1.0))
            .unwrap_err()
            .to_string(),
        "expected a value greater than or equal to 0 at $, got number"
    );
    assert_eq!(
        schema.decode(Unknown::Number(1.1)).unwrap_err().to_string(),
        "expected an integer at $, got number"
    );
    assert_eq!(schema.encode(&0.0), Unknown::Number(0.0));
    assert_eq!(
        schema.try_encode(&-1.0).unwrap_err().to_string(),
        "expected a value greater than or equal to 0 at $, got number"
    );
}

/// The four case rows are one shape: decoding normalizes, and encoding insists
/// the value was already normalized. Each row names its own sentence.
fn case_row(schema: Schema<String>, from: &str, to: &str, sentence: &str) {
    assert_eq!(schema.decode(text(from)), Ok(to.to_string()));
    assert_eq!(schema.encode(&to.to_string()), text(to));
    assert_eq!(
        schema
            .try_encode(&from.to_string())
            .unwrap_err()
            .to_string(),
        format!("expected {sentence} at $, got string")
    );
    // Anything already in the normal form has a wire form, so the check is the
    // normal form and not the row's own pair of strings.
    assert_eq!(schema.try_encode(&from.to_string()).is_err(), from != to);
}

#[test]
fn capitalize_normalizes_on_decode_and_insists_on_encode() {
    // :10194 "Capitalize".
    case_row(
        Schema::capitalize(),
        "abc",
        "Abc",
        "a string with the first character in uppercase",
    );
}

#[test]
fn uncapitalize_normalizes_on_decode_and_insists_on_encode() {
    // :10218 "Uncapitalize".
    case_row(
        Schema::uncapitalize(),
        "Abc",
        "abc",
        "a string with the first character in lowercase",
    );
}

#[test]
fn lowercase_normalizes_on_decode_and_insists_on_encode() {
    // :10242 "Lowercase".
    case_row(
        Schema::lowercase(),
        "ABC",
        "abc",
        "a string with all characters in lowercase",
    );
}

#[test]
fn uppercase_normalizes_on_decode_and_insists_on_encode() {
    // :10266 "Uppercase".
    case_row(
        Schema::uppercase(),
        "abc",
        "ABC",
        "a string with all characters in uppercase",
    );
}

#[test]
fn a_versioned_uuid_check_refuses_the_nil_and_max_uuids() {
    // :10361 "isUUID version-specific checks reject nil and max UUIDs": the
    // version nibble decides, so the two special forms the unversioned check
    // admits are refused here.
    let schema = Schema::is_uuid(Some(4));
    assert_eq!(
        schema.decode(text("00000000-0000-4000-8000-000000000001")),
        Ok("00000000-0000-4000-8000-000000000001".to_string())
    );
    for refused in [
        "00000000-0000-0000-0000-000000000000",
        "ffffffff-ffff-ffff-ffff-ffffffffffff",
    ] {
        assert_eq!(
            message(&schema, text(refused)),
            "expected a UUID v4 at $, got string",
            "{refused}"
        );
    }
    // The unversioned check admits exactly those two.
    let any = Schema::is_uuid(None);
    for accepted in [
        "00000000-0000-0000-0000-000000000000",
        "ffffffff-ffff-ffff-ffff-ffffffffffff",
        "FFFFFFFF-FFFF-FFFF-FFFF-FFFFFFFFFFFF",
        "00000000-0000-4000-8000-000000000001",
    ] {
        assert_eq!(
            any.decode(text(accepted)),
            Ok(accepted.to_string()),
            "{accepted}"
        );
    }
    assert_eq!(
        message(&any, text("00000000-0000-0000-0000-000000000001")),
        "expected a UUID at $, got string"
    );
}
