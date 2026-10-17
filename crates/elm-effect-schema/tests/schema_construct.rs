//! Schema construction fence: `make` fills constructor defaults while decode
//! and encode ignore them. Derives from the `withConstructorDefault` family in
//! packages/effect/test/schema/Schema.test.ts.
//!
//! Values in this crate's `Unknown` are the wire form (decoding a transformed
//! value returns its encoded shape, as `ffs_tuples_union_by_shape` shows), so a
//! constructed `FiniteFromString` field reads as its text form while the
//! default is proven by the fill itself: without the default `make` fails.

use std::collections::BTreeMap;
use std::sync::Arc;

use elm_effect_schema::{Field, Schema, SchemaError, Unknown};

fn map(pairs: &[(&str, Unknown)]) -> BTreeMap<String, Unknown> {
    pairs
        .iter()
        .map(|(key, value)| (key.to_string(), value.clone()))
        .collect()
}

fn obj(pairs: &[(&str, Unknown)]) -> Unknown {
    Unknown::Object(map(pairs))
}

fn number(value: f64) -> Unknown {
    Unknown::Number(value)
}

fn arr(items: Vec<Unknown>) -> Unknown {
    Unknown::Array(items)
}

fn text(value: &str) -> Unknown {
    Unknown::Text(value.to_string())
}

/// The pin's `FiniteFromString.pipe(withConstructorDefault(Effect.succeed(-1)))`.
fn finite_defaulting_to_minus_one() -> Schema<Unknown> {
    Schema::as_unknown(Schema::finite_from_string().with_constructor_default(|| Ok(number(-1.0))))
}

/// Schema.test.ts:3733 `should not apply defaults when decoding / encoding`.
#[test]
fn constructor_default_does_not_fill_on_decode_or_encode() {
    let field = Schema::as_unknown(
        Schema::string().with_constructor_default(|| Ok(Unknown::Text("a".to_string()))),
    );
    let schema = Schema::struct_shape(vec![Field::optional_key("a", field)]);

    assert_eq!(
        schema.decode(Unknown::Object(BTreeMap::new())).unwrap(),
        map(&[])
    );
    assert_eq!(schema.try_encode(&map(&[])).unwrap(), obj(&[]));
}

/// Schema.test.ts:3746 `should apply constructor default when the field is not
/// present`, and the sync/async Effect spellings at 3811 and 3822, whose only
/// observable result is the same fill.
#[test]
fn constructor_default_fills_an_absent_field_on_make() {
    let schema = Schema::struct_shape(vec![Field::required("a", finite_defaulting_to_minus_one())]);

    assert_eq!(
        schema.make(map(&[("a", number(1.0))])).unwrap(),
        map(&[("a", text("1"))])
    );
    assert_eq!(schema.make(map(&[])).unwrap(), map(&[("a", text("-1"))]));
    // Decoding the same input stays strict: construction is the only path
    // where the default applies.
    assert!(schema.decode(Unknown::Object(map(&[]))).is_err());
}

/// Schema.test.ts:3768 `Struct` under `nested defaults`.
#[test]
fn nested_constructor_defaults_fill_recursively() {
    let inner = Schema::struct_shape(vec![Field::required("b", finite_defaulting_to_minus_one())]);
    let outer = Schema::struct_shape(vec![Field::required(
        "a",
        Schema::as_unknown(inner.with_constructor_default(|| Ok(obj(&[])))),
    )]);

    assert_eq!(
        outer
            .make(map(&[("a", obj(&[("b", number(1.0))]))]))
            .unwrap(),
        map(&[("a", obj(&[("b", text("1"))]))])
    );
    assert_eq!(
        outer.make(map(&[("a", obj(&[]))])).unwrap(),
        map(&[("a", obj(&[("b", text("-1"))]))])
    );
    assert_eq!(
        outer.make(map(&[])).unwrap(),
        map(&[("a", obj(&[("b", text("-1"))]))])
    );
}

/// Schema.test.ts:3838 `Effect failing with SchemaIssue propagates as parse
/// failure`.
#[test]
fn constructor_default_failure_lands_on_the_field_path() {
    let schema = Schema::struct_shape(vec![Field::required(
        "a",
        Schema::as_unknown(Schema::finite_from_string().with_constructor_default(|| {
            Err(SchemaError::DefaultFailed {
                path: "$".to_string(),
                message: "ctor default failed".to_string(),
            })
        })),
    )]);

    // A present field never runs the default.
    assert!(schema.make(map(&[("a", number(1.0))])).is_ok());

    let error = schema.make(map(&[])).unwrap_err();
    assert!(error.to_string().contains("ctor default failed"));
    assert!(error.to_string().contains("$.a"));
}

/// Schema.test.ts:3855 `Effect failing via another schema's makeEffect
/// propagates as parse failure`.
#[test]
fn a_default_that_calls_another_failing_make_keeps_its_path() {
    let inner = Arc::new(Schema::struct_shape(vec![Field::required(
        "n",
        Schema::as_unknown(Schema::finite_from_string().refine("greater than 0", |n| *n > 0.0)),
    )]));
    let field = Arc::clone(&inner);
    let schema = Schema::struct_shape(vec![Field::required(
        "a",
        Schema::as_unknown(
            Schema::finite_from_string().with_constructor_default(move || {
                // The pin's default is `Inner.makeEffect({ n: -1 }).pipe(map((s) => s.n))`,
                // so the inner make's failure is the default's failure.
                let built = field.make(map(&[("n", number(-1.0))]))?;
                Ok(built.get("n").cloned().unwrap_or(Unknown::Null))
            }),
        ),
    )]);

    assert!(schema.make(map(&[("a", number(1.0))])).is_ok());

    let error = schema.make(map(&[])).unwrap_err();
    assert!(error.to_string().contains("$.a.n"));
}

/// Schema.test.ts:3906 `Tuple & Some`.
#[test]
fn tuple_constructor_default_fills_a_missing_slot() {
    let schema = Schema::tuple_of(vec![finite_defaulting_to_minus_one()]);

    assert_eq!(
        schema.make(vec![number(1.0)]).unwrap(),
        arr(vec![text("1")])
    );
    assert_eq!(schema.make(vec![]).unwrap(), arr(vec![text("-1")]));
}

/// Schema.test.ts:3917 `nested defaults (Struct)` under `Tuple`.
#[test]
fn nested_tuple_struct_defaults_fill_recursively() {
    let inner = Schema::struct_shape(vec![Field::required("b", finite_defaulting_to_minus_one())]);
    let schema = Schema::tuple_of(vec![Schema::as_unknown(
        inner.with_constructor_default(|| Ok(obj(&[]))),
    )]);

    assert_eq!(
        schema.make(vec![obj(&[("b", number(1.0))])]).unwrap(),
        arr(vec![obj(&[("b", text("1"))])])
    );
    assert_eq!(
        schema.make(vec![obj(&[])]).unwrap(),
        arr(vec![obj(&[("b", text("-1"))])])
    );
    assert_eq!(
        schema.make(vec![]).unwrap(),
        arr(vec![obj(&[("b", text("-1"))])])
    );
}

/// Schema.test.ts:3933 `inner defaults (Tuple)`.
#[test]
fn nested_tuple_defaults_fill_inner_slots() {
    let inner = Schema::tuple_of(vec![finite_defaulting_to_minus_one()]);
    let schema = Schema::tuple_of(vec![Schema::as_unknown(inner)]);

    assert_eq!(
        schema.make(vec![arr(vec![number(1.0)])]).unwrap(),
        arr(vec![arr(vec![text("1")])])
    );
    assert_eq!(
        schema.make(vec![arr(vec![])]).unwrap(),
        arr(vec![arr(vec![text("-1")])])
    );
}

/// Schema.test.ts:3948 `nested defaults (Tuple)`.
#[test]
fn a_defaulted_tuple_fills_its_own_missing_elements() {
    let inner = Schema::tuple_of(vec![finite_defaulting_to_minus_one()])
        .with_constructor_default(|| Ok(Unknown::Array(Vec::new())));
    let schema = Schema::tuple_of(vec![Schema::as_unknown(inner)]);

    assert_eq!(
        schema.make(vec![arr(vec![number(1.0)])]).unwrap(),
        arr(vec![arr(vec![text("1")])])
    );
    assert_eq!(
        schema.make(vec![arr(vec![])]).unwrap(),
        arr(vec![arr(vec![text("-1")])])
    );
    assert_eq!(
        schema.make(vec![]).unwrap(),
        arr(vec![arr(vec![text("-1")])])
    );
}
