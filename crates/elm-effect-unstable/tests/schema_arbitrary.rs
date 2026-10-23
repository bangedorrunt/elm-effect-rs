//! The schema-to-arbitrary derivation: `Arbitrary.schema` over the port's own
//! `Schema`.
//!
//! The rows behind this fence are the ones that assert a *refusal*: a bound
//! that is NaN, an integer bound outside the safe range, and an infinite
//! interval excluded by a finite constraint all derive to nothing. The same
//! derivation is proved to construct, because a refusal-only implementation
//! would be the shim this replaced.

use elm_effect_schema::{Schema, Unknown};
use elm_effect_unstable::arbitrary::{schema, Gen};

/// `Number.check(...)` with one check, in the order the pin applies it.
fn checked(base: Schema<Unknown>, name: &str, args: Vec<String>) -> Schema<Unknown> {
    Schema::checked(
        base,
        name,
        args.iter().map(String::as_str).collect(),
        Vec::new(),
    )
}

fn number() -> Schema<Unknown> {
    Schema::as_unknown(Schema::<f64>::number())
}

fn int() -> Schema<Unknown> {
    Schema::as_unknown(Schema::<f64>::int())
}

fn between(base: Schema<Unknown>, minimum: f64, maximum: f64) -> Schema<Unknown> {
    checked(
        base,
        "isBetween",
        vec![minimum.to_string(), maximum.to_string()],
    )
}

fn at_least(base: Schema<Unknown>, bound: f64) -> Schema<Unknown> {
    checked(base, "isGreaterThanOrEqualTo", vec![bound.to_string()])
}

fn message(error: &elm_effect_unstable::arbitrary::DeriveError) -> String {
    error.to_string()
}

/// Arbitrary.test.ts:1627 `rejects NaN numeric bounds`: a NaN on either side
/// has no inhabitant, for numbers and integers alike.
#[test]
fn rejects_nan_numeric_bounds() {
    let error = schema(&between(number(), f64::NAN, 1.0))
        .expect_err("a NaN minimum has no number to derive");
    assert!(
        message(&error).contains("Unable to derive an arbitrary for number constraints"),
        "{error}"
    );

    let error = schema(&between(int(), 0.0, f64::NAN))
        .expect_err("a NaN maximum has no integer to derive");
    assert!(
        message(&error).contains("Unable to derive an arbitrary for integer constraints"),
        "{error}"
    );
}

/// Arbitrary.test.ts:1817 `rejects infinite intervals excluded by a finite
/// constraint`: `isFinite` together with a `>= +Infinity` bound is empty.
#[test]
fn rejects_infinite_intervals_excluded_by_a_finite_constraint() {
    let finite = checked(at_least(number(), f64::INFINITY), "isFinite", Vec::new());
    let error = schema(&finite).expect_err("no finite number is at least infinity");
    assert!(
        message(&error).contains("Unable to derive an arbitrary for number constraints"),
        "{error}"
    );

    // Without the finite constraint the same bound is a legal one-sided
    // interval, and the one value it admits is the infinity itself.
    let open = schema(&at_least(number(), f64::INFINITY)).expect("an open bound derives");
    assert!(open.sample_n(7, 8).iter().all(|value| matches!(
        value,
        Unknown::Number(number) if *number == f64::INFINITY
    )));
}

/// Arbitrary.test.ts:1616 `rejects integer bounds outside the safe range`.
#[test]
fn rejects_integer_bounds_outside_the_safe_range() {
    const MAX_SAFE: f64 = 9_007_199_254_740_991.0;
    let above = checked(
        int(),
        "isGreaterThan",
        vec![MAX_SAFE.to_string()],
    );
    assert!(
        schema(&above).is_err(),
        "a minimum above the safe range has no inhabitant"
    );

    let below = checked(
        int(),
        "isLessThan",
        vec![(-MAX_SAFE).to_string()],
    );
    assert!(
        schema(&below).is_err(),
        "a maximum below the safe range has no inhabitant"
    );

    // The safe range itself is still derivable, edges included.
    assert!(schema(&between(int(), -MAX_SAFE, MAX_SAFE)).is_ok());
}

/// The derivation is not refusal-only: a pushed-down bound holds in every
/// sample, which is the property the pin's constraint pushing exists for.
#[test]
fn pushed_down_number_bounds_hold_in_every_sample() {
    let bounded = schema(&between(number(), 2.0, 5.0)).expect("a bounded number derives");
    let samples = bounded.sample_n(11, 200);
    assert_eq!(samples.len(), 200);
    for value in samples {
        let Unknown::Number(number) = value else {
            panic!("a Number schema samples numbers, got {value:?}");
        };
        assert!(
            (2.0..=5.0).contains(&number),
            "{number} is outside the bounds the schema carries"
        );
    }
}

/// A check the derivation cannot push into a generator refuses the schema
/// rather than sampling values it would have to filter out afterwards.
#[test]
fn refuses_a_check_it_cannot_push_down() {
    let multiples = checked(number(), "isMultipleOf", vec!["3".to_string()]);
    let error = schema(&multiples).expect_err("isMultipleOf has no generator here");
    assert!(
        message(&error).contains("number constraints"),
        "{error}"
    );
}

/// Structs, arrays and strings derive through their own shapes.
#[test]
fn derives_structs_arrays_and_strings() {
    let record = Schema::struct_([
        ("name", Schema::as_unknown(Schema::string())),
        ("count", Schema::as_unknown(Schema::<f64>::int())),
    ]);
    let derived = schema(&record).expect("a struct of scalars derives");
    for value in derived.sample_n(3, 20) {
        assert_eq!(value.field_names(), vec!["name", "count"]);
        assert!(matches!(value.get("name"), Some(Unknown::Text(_))));
        assert!(matches!(value.get("count"), Some(Unknown::Number(_))));
    }

    let list = Schema::<Vec<String>>::array(Schema::string());
    let derived = schema(&list).expect("an array derives");
    for value in derived.sample_n(5, 10) {
        assert!(value.len() <= 3, "a derivation has no size signal: {value:?}");
    }

    // An unbounded string is as long as the derivation's size signal, which is
    // a fixed value here because the port has no per-call size.
    let derived = schema(&Schema::<String>::string()).expect("a string derives");
    assert!(derived.sample_n(9, 10).iter().all(|value| value.len() <= 32));
}

/// A shape with no derivation says which one, at the path it was walking.
#[test]
fn names_the_shape_it_cannot_derive() {
    let json = Schema::<Unknown>::json();
    let error = schema(&json).expect_err("Json has no derived generator");
    assert!(message(&error).contains("the `Json` shape"), "{error}");

    let nested = Schema::struct_([(
        "rows",
        Schema::as_unknown(Schema::<Unknown>::json()),
    )]);
    let error = schema(&nested).expect_err("the field's shape has no generator");
    assert!(message(&error).contains("$.rows"), "{error}");
}

/// The generator type is the port's `Gen`, so the derivation composes with
/// everything else here (`map`, `filter`, `sample_n`).
#[test]
fn the_derived_generator_composes() {
    let bounded = schema(&between(int(), 0.0, 9.0)).expect("derives");
    let doubled: Gen<String> = bounded.map(|value| match value {
        Unknown::Number(number) => format!("{:02}", number as i64 * 2),
        other => format!("{other:?}"),
    });
    for value in doubled.sample_n(2, 10) {
        let parsed: i64 = value.parse().expect("the map renders digits");
        assert!((0..=18).contains(&parsed), "{parsed}");
        assert!(parsed % 2 == 0);
    }
}
