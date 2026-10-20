//! `Schema.test.ts` "Enum" rows: the finite-member guard, the names a set
//! exposes, and the three spellings (numeric, string, const) that differ only
//! in the values they name.
//!
//!     cargo test -p elm-effect-schema --test schema_enumeration

use elm_effect_schema::{Schema, SchemaError, Unknown};

fn text(value: &str) -> Unknown {
    Unknown::Text(value.to_string())
}

fn number(value: f64) -> Unknown {
    Unknown::Number(value)
}

/// The "expected" side of a refusal, which is the whole of the pin's message
/// once its own `Expected ` prefix is added. Read off the variant rather than
/// out of the rendered text: the port renders it lower case and appends where
/// the value arrived, and neither belongs in the pin's own sentence.
fn expected_side(error: &SchemaError) -> String {
    match error {
        SchemaError::TypeMismatchDynamic { expected, .. } => expected.clone(),
        other => panic!("not a union refusal: {other}"),
    }
}

/// Schema.test.ts:7810 — "rejects non-finite numeric values". The pin throws
/// while the enum is built, so the refusal is a panic here too.
#[test]
fn a_non_finite_member_is_refused_at_build_time() {
    for (value, printed) in [
        (f64::NAN, "NaN"),
        (f64::INFINITY, "Infinity"),
        (f64::NEG_INFINITY, "-Infinity"),
    ] {
        let panicked = std::panic::catch_unwind(|| {
            Schema::enumeration(vec![("value", number(value))]);
        });
        let message = panicked
            .err()
            .and_then(|payload| {
                payload
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| payload.downcast_ref::<&str>().map(|s| (*s).to_string()))
            })
            .unwrap_or_else(|| panic!("{printed} must be refused"));
        assert_eq!(
            message,
            format!("A numeric enum value must be finite, got {printed}")
        );
    }

    // A finite member of the same shape still builds.
    assert!(
        Schema::enumeration(vec![("value", number(1.0))])
            .schema()
            .decode(number(1.0))
            .is_ok()
    );
}

/// Schema.test.ts:7819 — "enums should be exposed". A numeric enum's names are
/// reachable in declaration order.
#[test]
fn the_names_are_exposed_in_declaration_order() {
    let fruits = Schema::enumeration(vec![("Apple", number(0.0)), ("Banana", number(1.0))]);

    assert_eq!(
        fruits.enums(),
        &[
            ("Apple".to_string(), number(0.0)),
            ("Banana".to_string(), number(1.0)),
        ]
    );
    assert_eq!(fruits.value("Apple"), Some(&number(0.0)));
    assert_eq!(fruits.value("Banana"), Some(&number(1.0)));
    assert_eq!(fruits.value("Cantaloupe"), None);
}

/// Schema.test.ts:7829 — "Numeric enum". A member and the number it holds are
/// the same wire value, and the refusal names every member.
#[test]
fn a_numeric_enum_decodes_its_members_and_refuses_the_rest() {
    let fruits = Schema::enumeration(vec![("Apple", number(0.0)), ("Banana", number(1.0))]);
    let schema = fruits.schema();

    assert!(schema.decode(number(0.0)).is_ok());
    assert!(schema.decode(number(1.0)).is_ok());
    assert_eq!(
        expected_side(&schema.decode(number(3.0)).unwrap_err()),
        "0 | 1"
    );

    // Encoding is the identity: the wire form is the member itself.
    for value in [number(0.0), number(1.0)] {
        assert_eq!(schema.try_encode(&value).unwrap(), value);
    }
}

/// Schema.test.ts:7853 — "String enum". A set may mix spellings, and two names
/// may hold the same value; the refusal keeps the declaration order.
#[test]
fn a_string_enum_mixes_spellings_and_keeps_the_order() {
    let fruits = Schema::enumeration(vec![
        ("Apple", text("apple")),
        ("Banana", text("banana")),
        ("Cantaloupe", number(0.0)),
    ]);
    let schema = fruits.schema();

    assert_eq!(fruits.enums().len(), 3);
    for accepted in [text("apple"), text("banana"), number(0.0)] {
        assert!(schema.decode(accepted.clone()).is_ok(), "{accepted:?}");
    }
    // The name is not a wire value: `"Cantaloupe"` is refused exactly as any
    // other string outside the set is.
    assert_eq!(
        expected_side(&schema.decode(text("Cantaloupe")).unwrap_err()),
        "\"apple\" | \"banana\" | 0"
    );

    assert_eq!(schema.try_encode(&text("apple")).unwrap(), text("apple"));
    assert_eq!(schema.try_encode(&number(0.0)).unwrap(), number(0.0));
}

/// Schema.test.ts:7881 — "Const enum". The values come from a `const` object
/// rather than a TypeScript enum, which changes nothing on the wire.
#[test]
fn a_const_enum_decodes_like_the_other_two() {
    let fruits = Schema::enumeration(vec![
        ("Apple", text("apple")),
        ("Banana", text("banana")),
        ("Cantaloupe", number(3.0)),
    ]);
    let schema = fruits.schema();

    for accepted in [text("apple"), text("banana"), number(3.0)] {
        assert!(schema.decode(accepted.clone()).is_ok(), "{accepted:?}");
    }
    assert_eq!(
        expected_side(&schema.decode(text("Cantaloupe")).unwrap_err()),
        "\"apple\" | \"banana\" | 3"
    );
    assert_eq!(fruits.value("Cantaloupe"), Some(&number(3.0)));
}

/// An empty set names nothing, so it admits nothing rather than accepting
/// every value. Port-local: the pin's Enum is built from a non-empty object.
#[test]
fn an_empty_set_admits_nothing() {
    let empty = Schema::enumeration(Vec::new());
    assert!(empty.enums().is_empty());
    assert!(empty.schema().decode(number(0.0)).is_err());
    assert!(empty.schema().decode(text("a")).is_err());
}
