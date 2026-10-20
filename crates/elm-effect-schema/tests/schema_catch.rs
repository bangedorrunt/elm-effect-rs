//! `Schema.test.ts` "catchDecoding"/"catchEncoding"/"middleware*" rows: a
//! refused direction falls back to a value the caller supplies, and a
//! middleware hook sees every value on one side and may refuse it.
//!
//!     cargo test -p elm-effect-schema --test schema_catch
//!
//! The pin's fallbacks are Effects, which is how it reaches a service (`*WithContext`,
//! `providing a service`) and how an `async fallback` row delays one. This
//! port's parse is synchronous and has no context argument, so those rows stay
//! unported; what they would add over the sync rows is the delivery mechanism.
//!
//! The schemas here validate on BOTH axes, which `as_unknown` alone does not
//! (re-typing is a passthrough on the way out). A row that asserts what the
//! ENCODE side refuses needs that validation, so it is built with
//! `try_convert` over the re-typed base.

use elm_effect_schema::{Schema, SchemaError, Unknown};

fn text(value: &str) -> Unknown {
    Unknown::Text(value.to_string())
}

/// `Schema.String` at the `Unknown` level, refusing a non-text value on either
/// axis.
fn wire_string() -> Schema<Unknown> {
    Schema::try_convert(
        Schema::as_unknown(Schema::string()),
        |value: Unknown| Ok(value),
        |value: &Unknown| match value {
            Unknown::Text(_) => Ok(value.clone()),
            other => Err(SchemaError::TypeMismatch {
                path: "$".to_string(),
                expected: "string",
                got: other.kind(),
            }),
        },
    )
}

/// `Schema.Number` at the `Unknown` level, the same both-axes rule.
fn wire_number() -> Schema<Unknown> {
    Schema::try_convert(
        Schema::as_unknown(Schema::number()),
        |value: Unknown| Ok(value),
        |value: &Unknown| match value {
            Unknown::Number(_) => Ok(value.clone()),
            other => Err(SchemaError::TypeMismatch {
                path: "$".to_string(),
                expected: "number",
                got: other.kind(),
            }),
        },
    )
}

/// Schema.test.ts:7908 `catchDecoding` `sync fallback`: a refused decode
/// answers the fallback, and a check applied on top still has the last word,
/// because the fallback travels the chain like any other value. The encode side
/// has no fallback, so it still refuses.
#[test]
fn a_refused_decode_falls_back_and_the_check_still_decides() {
    let schema = Schema::catch_decoding(wire_string(), |_error| Some(text("b"))).refine(
        "a value with a length of at least 1",
        |value: &Unknown| matches!(value, Unknown::Text(text) if !text.is_empty()),
    );

    assert_eq!(schema.decode(text("a")).unwrap(), text("a"));
    assert_eq!(
        schema.decode(Unknown::Null).unwrap(),
        text("b"),
        "the fallback answers for the refusal"
    );
    let refused = schema
        .decode(text(""))
        .expect_err("the check decides the fallback too");
    assert!(
        refused.to_string().contains("length of at least 1"),
        "{refused}"
    );

    // Encoding is untouched: the fallback is about the wire coming in.
    assert_eq!(schema.try_encode(&text("a")).unwrap(), text("a"));
    // On the way out the check runs before the base does, so either one may be
    // the refusal that lands; what matters is that no fallback answers it.
    assert!(
        schema.try_encode(&Unknown::Null).is_err(),
        "no fallback on the way out"
    );
}

/// Schema.test.ts:7998 `catchEncoding` `sync fallback`: an encode the base
/// refuses falls back to the wire value the caller supplies, and a middleware
/// hook on the way out still decides what leaves, the fallback included.
#[test]
fn a_refused_encode_falls_back_to_a_wire_value() {
    let schema = Schema::middleware_encoding(
        Schema::catch_encoding(wire_number(), |_error| Some(Unknown::Number(0.0))),
        |wire: Unknown| match wire {
            Unknown::Number(value) if value.fract() == 0.0 => Ok(Unknown::Number(value)),
            other => Err(SchemaError::TypeMismatch {
                path: "$".to_string(),
                expected: "an integer",
                got: other.kind(),
            }),
        },
    );

    assert_eq!(
        schema.try_encode(&Unknown::Number(1.0)).unwrap(),
        Unknown::Number(1.0)
    );
    assert_eq!(
        schema.try_encode(&Unknown::Null).unwrap(),
        Unknown::Number(0.0),
        "the fallback answers for the refusal"
    );
    // A value the wire cannot hold is refused at the wire refinement, which is
    // where the pin's `encodeTo(Schema.Int)` sits.
    let refused = schema
        .try_encode(&Unknown::Number(1.2))
        .expect_err("a fraction is not an integer on the wire");
    assert!(refused.to_string().contains("integer"), "{refused}");
}

/// Schema.test.ts:7983 `middlewareDecoding` `forced failure`: the hook sees the
/// input before the base does and refuses it with its own message, so a value
/// the base would have accepted still fails.
#[test]
fn a_decoding_middleware_can_refuse_what_the_base_accepts() {
    let schema = Schema::middleware_decoding(wire_string(), |_input: Unknown| {
        Err(SchemaError::FilterFailed {
            path: "$".to_string(),
            message: "my message".to_string(),
        })
    });

    let refused = schema
        .decode(text("1"))
        .expect_err("the hook refuses every input")
        .to_string();
    assert_eq!(refused, "my message at $");
}

/// Schema.test.ts:8084 `middlewareEncoding` `forced failure`: the mirror on the
/// encode side.
#[test]
fn an_encoding_middleware_can_refuse_what_the_base_accepts() {
    let schema = Schema::middleware_encoding(wire_string(), |_wire: Unknown| {
        Err(SchemaError::FilterFailed {
            path: "$".to_string(),
            message: "my message".to_string(),
        })
    });

    assert_eq!(
        schema.decode(text("1")).unwrap(),
        text("1"),
        "the encode hook does not touch decoding"
    );
    let refused = schema
        .try_encode(&text("1"))
        .expect_err("the hook refuses every wire value")
        .to_string();
    assert_eq!(refused, "my message at $");
}
