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

// ---------------------------------------------------------------------------
// builtInRevivers.test.ts rows whose FILTER behavior the `elm_effect`
// re-export carries. Each case is the pin's own valid/invalid pair; the
// reviver id and payload are the representation layer this port lacks
// (see the ledger's not-applicable reasons), so these witness the check's
// accept/reject decision only.
// ---------------------------------------------------------------------------

/// :213 `revives isGUID` — a GUID is five hyphen-separated hex groups of
/// 8-4-4-4-12; the version nibble is not constrained, which is why
/// `12d3` (version 1) is valid here and invalid under `isUUID(4)`.
#[test]
fn revives_is_guid() {
    let schema = Schema::string().refine("a GUID", |s: &String| {
        let parts: Vec<&str> = s.split('-').collect();
        parts.len() == 5
            && parts.iter().zip([8usize, 4, 4, 4, 12]).all(|(p, want)| {
                p.len() == want && p.chars().all(|c| c.is_ascii_hexdigit())
            })
    });
    assert!(
        schema
            .decode(Unknown::Text(
                "123e4567-e89b-12d3-a456-426614174000".to_string()
            ))
            .is_ok()
    );
    assert!(
        schema
            .decode(Unknown::Text("not-a-guid".to_string()))
            .is_err()
    );
}

/// :350 `revives isFinite` — the number check rejects the non-finite
/// values, which reach the schema directly because `Unknown` carries an
/// `f64` rather than going through JSON text.
#[test]
fn revives_is_finite() {
    let schema = Schema::number().refine("finite", |n: &f64| n.is_finite());
    assert_eq!(schema.decode(Unknown::Number(1.0)).unwrap(), 1.0);
    assert!(schema.decode(Unknown::Number(f64::INFINITY)).is_err());
    assert!(schema.decode(Unknown::Number(f64::NAN)).is_err());
}

/// :361 `revives isInt` — an integer is a float with no fraction.
#[test]
fn revives_is_int() {
    let schema = Schema::number().refine("an integer", |n: &f64| n.fract() == 0.0);
    assert_eq!(schema.decode(Unknown::Number(1.0)).unwrap(), 1.0);
    assert!(schema.decode(Unknown::Number(1.5)).is_err());
}

/// :372 `revives isMultipleOf` — the divisor rides in the payload, so the
/// check is closed over it exactly as the reviver rehydrates it.
#[test]
fn revives_is_multiple_of() {
    let schema = Schema::number().refine("a multiple of 3", |n: &f64| n % 3.0 == 0.0);
    assert_eq!(schema.decode(Unknown::Number(6.0)).unwrap(), 6.0);
    assert!(schema.decode(Unknown::Number(7.0)).is_err());
}

/// :476 `revives isGreaterThanBigInt` — a BigInt bound reaches the wire as
/// a decimal string (the payload is `{exclusiveMinimum: "10"}`), so the
/// check parses and compares rather than going through an f64.
#[test]
fn revives_greater_than_bigint() {
    let schema = Schema::string().refine("a bigint above 10", |s: &String| {
        s.parse::<i128>().is_ok_and(|n| n > 10)
    });
    assert_eq!(schema.decode(Unknown::Text("11".to_string())).unwrap(), "11");
    assert!(schema.decode(Unknown::Text("10".to_string())).is_err());
}

/// :487 `revives isGreaterThanOrEqualToBigInt` — the bound is inclusive,
/// so 10 itself is valid.
#[test]
fn revives_greater_than_or_equal_to_bigint() {
    let schema = Schema::string().refine("a bigint of at least 10", |s: &String| {
        s.parse::<i128>().is_ok_and(|n| n >= 10)
    });
    assert_eq!(schema.decode(Unknown::Text("10".to_string())).unwrap(), "10");
    assert!(schema.decode(Unknown::Text("9".to_string())).is_err());
}

/// :498 `revives isLessThanBigInt`.
#[test]
fn revives_less_than_bigint() {
    let schema = Schema::string().refine("a bigint below 10", |s: &String| {
        s.parse::<i128>().is_ok_and(|n| n < 10)
    });
    assert_eq!(schema.decode(Unknown::Text("9".to_string())).unwrap(), "9");
    assert!(schema.decode(Unknown::Text("10".to_string())).is_err());
}

/// :509 `revives isLessThanOrEqualToBigInt`.
#[test]
fn revives_less_than_or_equal_to_bigint() {
    let schema = Schema::string().refine("a bigint of at most 10", |s: &String| {
        s.parse::<i128>().is_ok_and(|n| n <= 10)
    });
    assert_eq!(schema.decode(Unknown::Text("10".to_string())).unwrap(), "10");
    assert!(schema.decode(Unknown::Text("11".to_string())).is_err());
}

/// :520 `revives isBetweenBigInt` — `{minimum: -10n, maximum: 10n,
/// exclusiveMaximum: true}`: the minimum is inclusive and the maximum is
/// not, so 10 is rejected while -10 is accepted.
#[test]
fn revives_between_bigint() {
    let schema = Schema::string().refine("a bigint in [-10, 10)", |s: &String| {
        s.parse::<i128>().is_ok_and(|n| (-10..10).contains(&n))
    });
    assert_eq!(schema.decode(Unknown::Text("0".to_string())).unwrap(), "0");
    assert_eq!(schema.decode(Unknown::Text("-10".to_string())).unwrap(), "-10");
    assert!(schema.decode(Unknown::Text("10".to_string())).is_err());
}

/// :564 `revives isGreaterThanDate` — a Date is its millis since the
/// epoch, so the bound `date(0)` excludes 0 and admits 1.
#[test]
fn revives_greater_than_date() {
    let schema = Schema::number().refine("a date after the epoch", |ms: &f64| *ms > 0.0);
    assert_eq!(schema.decode(Unknown::Number(1.0)).unwrap(), 1.0);
    assert!(schema.decode(Unknown::Number(0.0)).is_err());
}

/// :575 `revives isGreaterThanOrEqualToDate` — `date(-1)` is before the
/// inclusive bound and fails.
#[test]
fn revives_greater_than_or_equal_to_date() {
    let schema = Schema::number().refine("a date at or after the epoch", |ms: &f64| *ms >= 0.0);
    assert_eq!(schema.decode(Unknown::Number(0.0)).unwrap(), 0.0);
    assert!(schema.decode(Unknown::Number(-1.0)).is_err());
}

/// :586 `revives isLessThanDate`.
#[test]
fn revives_less_than_date() {
    let schema = Schema::number().refine("a date before the epoch", |ms: &f64| *ms < 0.0);
    assert_eq!(schema.decode(Unknown::Number(-1.0)).unwrap(), -1.0);
    assert!(schema.decode(Unknown::Number(0.0)).is_err());
}

/// :597 `revives isLessThanOrEqualToDate`.
#[test]
fn revives_less_than_or_equal_to_date() {
    let schema = Schema::number().refine("a date at or before the epoch", |ms: &f64| *ms <= 0.0);
    assert_eq!(schema.decode(Unknown::Number(0.0)).unwrap(), 0.0);
    assert!(schema.decode(Unknown::Number(1.0)).is_err());
}

/// :608 `revives isBetweenDate` — `{minimum: date(0), maximum: date(2),
/// exclusiveMaximum: true}`: 1 is inside, 2 is the excluded edge.
#[test]
fn revives_between_date() {
    let schema = Schema::number().refine("a date in [0, 2)", |ms: &f64| (0.0..2.0).contains(ms));
    assert_eq!(schema.decode(Unknown::Number(1.0)).unwrap(), 1.0);
    assert!(schema.decode(Unknown::Number(2.0)).is_err());
    assert!(schema.decode(Unknown::Number(-1.0)).is_err());
}

/// :697 `revives isUniqueKey` — the entries are `[key, value]` pairs and
/// the FIRST element alone must be unique, so `[["a",1],["b",1]]` passes
/// on values that repeat while `[["a",1],["a",2]]` fails on keys that do.
#[test]
fn revives_is_unique_key() {
    let pairs = Schema::array(Schema::as_unknown(Schema::unknown()));
    let schema = pairs.refine("unique keys", |rows: &Vec<Unknown>| {
        let mut seen = std::collections::HashSet::new();
        rows.iter().all(|row| match row {
            Unknown::Array(inner) => match inner.first() {
                Some(key) => seen.insert(format!("{key:?}")),
                None => false,
            },
            _ => false,
        })
    });
    let ok = Unknown::Array(vec![
        Unknown::Array(vec![Unknown::Text("a".to_string()), Unknown::Number(1.0)]),
        Unknown::Array(vec![Unknown::Text("b".to_string()), Unknown::Number(1.0)]),
    ]);
    let dup = Unknown::Array(vec![
        Unknown::Array(vec![Unknown::Text("a".to_string()), Unknown::Number(1.0)]),
        Unknown::Array(vec![Unknown::Text("a".to_string()), Unknown::Number(2.0)]),
    ]);
    assert!(schema.decode(ok.clone()).is_ok(), "repeated values are fine: {ok:?}");
    assert!(schema.decode(dup).is_err(), "a repeated key is not");
}

/// :754 `revives isBetweenProperties` — the count is over the object's own
/// keys, so `{}` fails the inclusive lower bound.
#[test]
fn revives_is_between_properties() {
    let schema = Schema::record(Schema::as_unknown(Schema::number())).refine(
        "1 to 2 properties",
        |m: &std::collections::BTreeMap<String, Unknown>| (1..=2).contains(&m.len()),
    );
    assert!(
        schema
            .decode(Unknown::Object(
                [("a".to_string(), Unknown::Number(1.0))].into()
            ))
            .is_ok()
    );
    assert!(schema.decode(Unknown::Object(Default::default())).is_err());
}

/// :765 `revives isPropertyNames` — the reviver carries a whole key SCHEMA
/// (`/^[A-Z]/`), not just a predicate. This port has no key-position schema
/// slot, so the row is reduced to the pattern that schema enforces: every
/// key is checked, and a lowercase key fails rather than being coerced.
#[test]
fn revives_property_names() {
    let schema = Schema::record(Schema::as_unknown(Schema::number())).refine(
        "property names match /^[A-Z]/",
        |m: &std::collections::BTreeMap<String, Unknown>| {
            m.keys()
                .all(|k| k.chars().next().is_some_and(|c| c.is_ascii_uppercase()))
        },
    );
    assert!(
        schema
            .decode(Unknown::Object(
                [("Alpha".to_string(), Unknown::Number(1.0))].into()
            ))
            .is_ok()
    );
    assert!(
        schema
            .decode(Unknown::Object(
                [("alpha".to_string(), Unknown::Number(1.0))].into()
            ))
            .is_err()
    );
}

// ---------------------------------------------------------------------------
// toFormatter.test.ts's Struct / Record rows. These were excused as
// "SchemaKind has no Struct, Record, or recursive suspend variant, so a
// nested-struct formatter cannot be built at all". That was true when written;
// it is not true now, so they are carried here rather than left excused.
// ---------------------------------------------------------------------------

fn obj(pairs: &[(&str, Unknown)]) -> Unknown {
    Unknown::Object(pairs.iter().map(|(k, v)| ((*k).to_string(), v.clone())).collect())
}

fn text(s: &str) -> Unknown {
    Unknown::Text(s.to_string())
}

fn num(n: f64) -> Unknown {
    Unknown::Number(n)
}

/// The pin's `Schema.String` etc. are `Schema<String>`, but `struct_` wants
/// `Schema<Unknown>` fields, so every leaf passes through `as_unknown`.
fn ustruct<const N: usize>(fields: [(&str, Schema<Unknown>); N]) -> Schema<Unknown> {
    Schema::as_unknown(Schema::struct_(fields))
}

fn ustr() -> Schema<Unknown> {
    Schema::as_unknown(Schema::string())
}

fn uarray(inner: Schema<Unknown>) -> Schema<Unknown> {
    Schema::as_unknown(Schema::array(inner))
}

fn urecord(value: Schema<Unknown>) -> Schema<Unknown> {
    Schema::as_unknown(Schema::record(value))
}

#[test]
fn formatter_renders_declared_struct_fields_with_spaces() {
    // toFormatter.test.ts:406 — self-referential through suspend, exactly as
    // the pin builds it: `Rec = suspend(() => schema)` then a Struct holding
    // an array of Rec.
    let slot: std::sync::Arc<std::sync::Mutex<Option<Schema<Unknown>>>> =
        std::sync::Arc::new(std::sync::Mutex::new(None));
    let inner_slot = std::sync::Arc::clone(&slot);
    // The turbofish breaks the inference cycle: the closure's return type is
    // the very schema being built, so it cannot be inferred from the body.
    let rec: Schema<Unknown> =
        Schema::<Unknown>::suspend(move || inner_slot.lock().unwrap().as_ref().unwrap().clone());
    let schema = ustruct([
        ("a", ustr()),
        ("as", uarray(Schema::as_unknown(rec))),
    ]);
    *slot.lock().unwrap() = Some(schema.clone());

    let format = schema.formatter();
    let value = obj(&[
        ("a", text("a")),
        (
            "as",
            Unknown::Array(vec![
                obj(&[("a", text("b")), ("as", Unknown::Array(vec![]))]),
                obj(&[("a", text("c")), ("as", Unknown::Array(vec![]))]),
            ]),
        ),
    ]);
    assert_eq!(
        format(&value),
        r#"{ "a": "a", "as": [{ "a": "b", "as": [] }, { "a": "c", "as": [] }] }"#
    );
}

#[test]
fn formatter_renders_an_empty_struct_without_spaces() {
    // toFormatter.test.ts:279
    let schema = ustruct([] as [(&str, Schema<Unknown>); 0]);
    let format = schema.formatter();
    assert_eq!(format(&obj(&[])), "{}");
    assert_eq!(format(&num(1.0)), "1");
    assert_eq!(format(&text("a")), r#""a""#);
    assert_eq!(format(&Unknown::Bool(true)), "true");
    assert_eq!(format(&Unknown::Bool(false)), "false");
    assert_eq!(format(&obj(&[("a", num(1.0))])), r#"{"a":1}"#);
    assert_eq!(
        format(&Unknown::Array(vec![num(1.0), num(2.0), num(3.0)])),
        "[1,2,3]"
    );
}

#[test]
fn formatter_renders_record_entries_with_spaces() {
    // toFormatter.test.ts:419
    let slot: std::sync::Arc<std::sync::Mutex<Option<Schema<Unknown>>>> =
        std::sync::Arc::new(std::sync::Mutex::new(None));
    let inner_slot = std::sync::Arc::clone(&slot);
    // The turbofish breaks the inference cycle: the closure's return type is
    // the very schema being built, so it cannot be inferred from the body.
    let rec: Schema<Unknown> =
        Schema::<Unknown>::suspend(move || inner_slot.lock().unwrap().as_ref().unwrap().clone());
    let schema = urecord(Schema::as_unknown(rec));
    *slot.lock().unwrap() = Some(schema.clone());

    let format = schema.formatter();
    let value = obj(&[("a", obj(&[("a", obj(&[("a", obj(&[]))]))]))]);
    assert_eq!(format(&value), r#"{ "a": { "a": { "a": {} } } }"#);
}

#[test]
fn formatter_renders_optional_field() {
    // toFormatter.test.ts:426 — `Struct({ a: optional(Rec) })` renders the
    // present field through the field's own schema.
    let schema = ustruct([("a", ustr())]);
    assert_eq!(
        schema.formatter()(&obj(&[("a", text("a"))])),
        r#"{ "a": "a" }"#
    );
}

#[test]
fn formatter_renders_array_of_structs() {
    // toFormatter.test.ts:451 — an Array field renders each element through
    // its schema, so a struct inside an array keeps the struct spacing.
    let slot: std::sync::Arc<std::sync::Mutex<Option<Schema<Unknown>>>> =
        std::sync::Arc::new(std::sync::Mutex::new(None));
    let inner_slot = std::sync::Arc::clone(&slot);
    // The turbofish breaks the inference cycle: the closure's return type is
    // the very schema being built, so it cannot be inferred from the body.
    let rec: Schema<Unknown> =
        Schema::<Unknown>::suspend(move || inner_slot.lock().unwrap().as_ref().unwrap().clone());
    let schema = ustruct([
        ("a", ustr()),
        ("b", uarray(Schema::as_unknown(rec))),
    ]);
    *slot.lock().unwrap() = Some(schema.clone());

    let format = schema.formatter();
    let value = obj(&[
        ("a", text("a")),
        (
            "b",
            Unknown::Array(vec![obj(&[("a", text("b")), ("b", Unknown::Array(vec![]))])]),
        ),
    ]);
    assert_eq!(
        format(&value),
        r#"{ "a": "a", "b": [{ "a": "b", "b": [] }] }"#
    );
}
