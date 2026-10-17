//! Slice 9: `Schema.toEquivalence` (typed-side equivalence relations).
//! Ports packages/effect/test/schema/toEquivalence.test.ts cells by exact call.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use elm_effect_schema::class::class;
use elm_effect_schema::transform::date_schema;
use elm_effect_schema::{Field, Schema, Unknown};

fn text(value: &str) -> Unknown {
    Unknown::Text(value.to_string())
}

fn num(value: f64) -> Unknown {
    Unknown::Number(value)
}

fn obj(pairs: Vec<(&str, Unknown)>) -> Unknown {
    Unknown::Object(
        pairs
            .into_iter()
            .map(|(name, value)| (name.to_string(), value))
            .collect::<BTreeMap<_, _>>(),
    )
}

fn str_schema() -> Schema<Unknown> {
    Schema::as_unknown(Schema::string())
}

/// `Schema.Number.annotate({ toEquivalence })` / `overrideToEquivalence`: the
/// port's factory spelling, called once while the schema is built.
fn number_with(relation: fn() -> Arc<dyn Fn(&f64, &f64) -> bool + Send + Sync>) -> Schema<f64> {
    Schema::override_to_equivalence(Schema::number(), relation)
}

// toEquivalence.test.ts:25 — `Never` answers with `strictEqual` (`===`), so two
// distinct objects are NOT equivalent even though they are structurally equal.
#[test]
fn never_uses_strict_identity() {
    let equivalence = Schema::never().to_equivalence();
    let value = Unknown::Object(BTreeMap::new());
    assert!(equivalence(&value, &value));
    assert!(!equivalence(
        &Unknown::Object(BTreeMap::new()),
        &Unknown::Object(BTreeMap::new())
    ));
}

// toEquivalence.test.ts:33
#[test]
fn string_compares_by_value() {
    let equivalence = Schema::string().to_equivalence();
    assert!(equivalence(&"a".to_string(), &"a".to_string()));
    assert!(!equivalence(&"a".to_string(), &"b".to_string()));
}

// toEquivalence.test.ts:41
#[test]
fn tuple_refuses_a_non_array_on_either_side() {
    let schema = Schema::tuple_of(vec![str_schema(), Schema::as_unknown(Schema::number())]);
    let equivalence = schema.to_equivalence();
    assert!(!equivalence(&obj(vec![("a", text("a"))]), &Unknown::Null));
}

// toEquivalence.test.ts:47
#[test]
fn empty_tuple_is_equivalent_to_an_empty_tuple() {
    let equivalence = Schema::tuple_of(vec![]).to_equivalence();
    assert!(equivalence(
        &Unknown::Array(vec![]),
        &Unknown::Array(vec![])
    ));
}

// toEquivalence.test.ts:53
#[test]
fn tuple_compares_required_elements_positionally() {
    let schema = Schema::tuple_of(vec![str_schema(), Schema::as_unknown(Schema::number())]);
    let equivalence = schema.to_equivalence();
    let value = Unknown::Array(vec![text("a"), num(1.0)]);
    assert!(equivalence(
        &value,
        &Unknown::Array(vec![text("a"), num(1.0)])
    ));
    assert!(!equivalence(
        &value,
        &Unknown::Array(vec![text("b"), num(1.0)])
    ));
}

// toEquivalence.test.ts:60
#[test]
fn tuple_compares_optional_key_elements_by_presence() {
    let schema = Schema::tuple_of(vec![
        str_schema(),
        Schema::optional_key_shape(Schema::as_unknown(Schema::number())),
    ]);
    let equivalence = schema.to_equivalence();
    assert!(equivalence(
        &Unknown::Array(vec![text("a"), num(1.0)]),
        &Unknown::Array(vec![text("a"), num(1.0)])
    ));
    assert!(equivalence(
        &Unknown::Array(vec![text("a")]),
        &Unknown::Array(vec![text("a")])
    ));
    assert!(!equivalence(
        &Unknown::Array(vec![text("a"), num(1.0)]),
        &Unknown::Array(vec![text("b"), num(1.0)])
    ));
    assert!(!equivalence(
        &Unknown::Array(vec![text("a")]),
        &Unknown::Array(vec![text("b")])
    ));
}

// toEquivalence.test.ts:69 — an `optional` element position is absent-or-null.
// The port's optional element uses `Unknown::Null` where the pin's value is
// `undefined`, the same standing-in `Schema::void` makes for a value-less
// result.
#[test]
fn tuple_compares_optional_elements_by_presence() {
    let schema = Schema::tuple_of(vec![
        str_schema(),
        Schema::as_unknown(Schema::optional(Schema::number())),
    ]);
    let equivalence = schema.to_equivalence();
    let present = Unknown::Array(vec![text("a"), num(1.0)]);
    let absent = Unknown::Array(vec![text("a")]);
    let nulled = Unknown::Array(vec![text("a"), Unknown::Null]);
    assert!(equivalence(&present, &present.clone()));
    assert!(equivalence(&absent, &absent.clone()));
    assert!(equivalence(&nulled, &nulled.clone()));
    assert!(!equivalence(
        &present,
        &Unknown::Array(vec![text("b"), num(1.0)])
    ));
    assert!(!equivalence(&absent, &Unknown::Array(vec![text("b")])));
    assert!(!equivalence(
        &nulled,
        &Unknown::Array(vec![text("b"), Unknown::Null])
    ));
}

// toEquivalence.test.ts:81 — length is part of the relation, in both directions.
#[test]
fn array_compares_length_and_elements() {
    let equivalence = Schema::array(Schema::string()).to_equivalence();
    let value = vec!["a".to_string(), "b".to_string(), "c".to_string()];
    assert!(equivalence(&value, &value.clone()));
    assert!(!equivalence(
        &value,
        &vec!["a".to_string(), "b".to_string(), "d".to_string()]
    ));
    assert!(!equivalence(
        &value,
        &vec!["a".to_string(), "b".to_string()]
    ));
    assert!(!equivalence(
        &value,
        &vec![
            "a".to_string(),
            "b".to_string(),
            "c".to_string(),
            "d".to_string()
        ]
    ));
}

// toEquivalence.test.ts:148
#[test]
fn struct_refuses_a_non_record_on_either_side() {
    let schema = Schema::as_unknown(Schema::struct_shape(vec![Field::required(
        "a",
        str_schema(),
    )]));
    let equivalence = schema.to_equivalence();
    assert!(!equivalence(&obj(vec![("a", text("a"))]), &num(1.0)));
}

// toEquivalence.test.ts:154 — an empty struct is structurally equal to another.
#[test]
fn empty_struct_is_structurally_equal() {
    let schema = Schema::struct_shape(vec![]);
    let equivalence = schema.to_equivalence();
    let a = schema.decode(obj(vec![])).expect("empty struct decodes");
    assert!(equivalence(&a, &a));
    assert!(equivalence(
        &schema.decode(obj(vec![])).expect("decodes"),
        &schema.decode(obj(vec![])).expect("decodes")
    ));
}

// toEquivalence.test.ts:162
#[test]
fn struct_compares_required_fields() {
    let schema = Schema::struct_shape(vec![
        Field::required("a", str_schema()),
        Field::required("b", Schema::as_unknown(Schema::number())),
    ]);
    let equivalence = schema.to_equivalence();
    let value = schema
        .decode(obj(vec![("a", text("a")), ("b", num(1.0))]))
        .expect("decodes");
    assert!(equivalence(
        &value,
        &schema
            .decode(obj(vec![("a", text("a")), ("b", num(1.0))]))
            .expect("decodes")
    ));
    assert!(!equivalence(
        &value,
        &schema
            .decode(obj(vec![("a", text("b")), ("b", num(1.0))]))
            .expect("decodes")
    ));
    assert!(!equivalence(
        &value,
        &schema
            .decode(obj(vec![("a", text("a")), ("b", num(2.0))]))
            .expect("decodes")
    ));
}

// toEquivalence.test.ts:192 — an absent `optionalKey` field on both sides is
// equivalent; one absent and one present is not.
#[test]
fn struct_compares_optional_key_fields_by_presence() {
    let schema = Schema::struct_shape(vec![
        Field::required("a", str_schema()),
        Field::optional_key("b", Schema::as_unknown(Schema::number())),
    ]);
    let equivalence = schema.to_equivalence();
    let with_b = schema
        .decode(obj(vec![("a", text("a")), ("b", num(1.0))]))
        .expect("decodes");
    let without_b = schema.decode(obj(vec![("a", text("a"))])).expect("decodes");
    assert!(equivalence(
        &with_b,
        &schema
            .decode(obj(vec![("a", text("a")), ("b", num(1.0))]))
            .expect("decodes")
    ));
    assert!(equivalence(
        &without_b,
        &schema.decode(obj(vec![("a", text("a"))])).expect("decodes")
    ));
    assert!(!equivalence(
        &without_b,
        &schema.decode(obj(vec![("a", text("b"))])).expect("decodes")
    ));
    assert!(!equivalence(
        &with_b,
        &schema
            .decode(obj(vec![("a", text("b")), ("b", num(1.0))]))
            .expect("decodes")
    ));
    assert!(!equivalence(
        &with_b,
        &schema
            .decode(obj(vec![("a", text("a")), ("b", num(2.0))]))
            .expect("decodes")
    ));
}

// toEquivalence.test.ts:205 — an `optional` field is absent-or-null; both
// skip the inner schema, so an explicit `undefined` (`Unknown::Null` here)
// decodes to the same value as an absent key.
#[test]
fn struct_compares_optional_fields_by_presence() {
    let schema = Schema::struct_shape(vec![
        Field::required("a", str_schema()),
        Field::optional("b", Schema::as_unknown(Schema::number())),
    ]);
    let equivalence = schema.to_equivalence();
    let decode = |value: Unknown| schema.decode(value).expect("decodes");
    let with_b = decode(obj(vec![("a", text("a")), ("b", num(1.0))]));
    let absent_b = decode(obj(vec![("a", text("a"))]));
    let undefined_b = decode(obj(vec![("a", text("a")), ("b", Unknown::Null)]));
    assert!(equivalence(
        &with_b,
        &decode(obj(vec![("a", text("a")), ("b", num(1.0))]))
    ));
    assert!(equivalence(&absent_b, &decode(obj(vec![("a", text("a"))]))));
    assert!(equivalence(
        &undefined_b,
        &decode(obj(vec![("a", text("a")), ("b", Unknown::Null)]))
    ));
    assert!(!equivalence(
        &with_b,
        &decode(obj(vec![("a", text("b")), ("b", num(1.0))]))
    ));
    assert!(!equivalence(
        &with_b,
        &decode(obj(vec![("a", text("a")), ("b", num(2.0))]))
    ));
    assert!(!equivalence(&with_b, &undefined_b));
    assert!(!equivalence(&undefined_b, &with_b));
}

// toEquivalence.test.ts:221 — a class is its declared fields; an extra property
// an instance carries is not part of what the class describes.
#[test]
fn class_compares_declared_fields() {
    let schema = class("A", vec![("value", str_schema())]);
    let equivalence = schema.to_equivalence();
    let a = schema
        .make(obj(vec![("value", text("a"))]))
        .expect("constructs");
    let b = schema
        .make(obj(vec![("value", text("a"))]))
        .expect("constructs");
    assert!(equivalence(&a, &b));
    assert!(!equivalence(
        &a,
        &schema
            .make(obj(vec![("value", text("b"))]))
            .expect("constructs")
    ));
}

// toEquivalence.test.ts:236
#[test]
fn record_compares_entries_and_key_set() {
    let schema = Schema::record2(str_schema(), Schema::as_unknown(Schema::number()));
    let equivalence = schema.to_equivalence();
    let value = schema
        .decode(obj(vec![("a", num(1.0)), ("b", num(2.0))]))
        .expect("decodes");
    assert!(equivalence(
        &value,
        &schema
            .decode(obj(vec![("a", num(1.0)), ("b", num(2.0))]))
            .expect("decodes")
    ));
    assert!(!equivalence(
        &value,
        &schema
            .decode(obj(vec![("a", num(1.0)), ("b", num(3.0))]))
            .expect("decodes")
    ));
    assert!(!equivalence(
        &value,
        &schema
            .decode(obj(vec![("a", num(2.0)), ("b", num(2.0))]))
            .expect("decodes")
    ));
    assert!(!equivalence(
        &value,
        &schema
            .decode(obj(vec![("a", num(1.0)), ("b", num(2.0)), ("c", num(3.0))]))
            .expect("decodes")
    ));
    assert!(!equivalence(
        &schema
            .decode(obj(vec![("a", num(1.0)), ("b", num(2.0)), ("c", num(3.0))]))
            .expect("decodes"),
        &value
    ));
}

// toEquivalence.test.ts:254 — the key schema selects which entries the
// equivalence sees, so a key it refuses cannot make two records unequal.
#[test]
fn record_key_check_selects_compared_entries() {
    // `Schema.isPattern(/^a/)` reads back out of the JS RegExp literal its repr
    // prints, which is the spelling every key-pattern row uses.
    let key = Schema::checked(
        str_schema(),
        "isPattern",
        vec![&format!("new RegExp(\"{}\", \"u\")", "^a")],
        Vec::new(),
    );
    let schema = Schema::record2(key, Schema::as_unknown(Schema::number()));
    let equivalence = schema.to_equivalence();
    assert!(equivalence(
        &schema
            .decode(obj(vec![("a", num(1.0)), ("b", num(1.0))]))
            .expect("decodes"),
        &schema
            .decode(obj(vec![("a", num(1.0)), ("b", num(2.0))]))
            .expect("decodes")
    ));
    assert!(!equivalence(
        &schema.decode(obj(vec![("a", num(1.0))])).expect("decodes"),
        &schema.decode(obj(vec![("a", num(2.0))])).expect("decodes")
    ));
}

// toEquivalence.test.ts:293 — the relation is derived once, and the recursive
// body is reused for deeper values rather than re-derived per level.
#[test]
fn suspend_reuses_the_relation_for_deeper_values() {
    let derivations = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&derivations);
    let value = Schema::as_unknown(Schema::override_to_equivalence(
        Schema::number(),
        move || {
            counter.fetch_add(1, Ordering::SeqCst);
            Arc::new(|a: &f64, b: &f64| a == b)
        },
    ));
    fn node(value: &Schema<Unknown>) -> Schema<BTreeMap<String, Unknown>> {
        let inner = value.clone();
        Schema::struct_shape(vec![
            Field::required("value", value.clone()),
            Field::required(
                "children",
                Schema::as_unknown(Schema::array(Schema::as_unknown(
                    Schema::<Unknown>::suspend(move || Schema::as_unknown(node(&inner))),
                ))),
            ),
        ])
    }
    fn make(depth: i32, leaf: f64) -> Unknown {
        let value = if depth == 0 {
            num(leaf)
        } else {
            num(depth as f64)
        };
        let children = if depth == 0 {
            vec![]
        } else {
            vec![make(depth - 1, leaf)]
        };
        obj(vec![
            ("value", value),
            ("children", Unknown::Array(children)),
        ])
    }

    let equivalence = Schema::as_unknown(node(&value)).to_equivalence();
    assert_eq!(derivations.load(Ordering::SeqCst), 1);
    for depth in [1, 8, 32] {
        assert!(equivalence(&make(depth, 0.0), &make(depth, 0.0)));
        assert!(!equivalence(&make(depth, 0.0), &make(depth, -1.0)));
        assert_eq!(derivations.load(Ordering::SeqCst), 1);
    }
}

// toEquivalence.test.ts:323
#[test]
fn suspend_compares_a_recursive_schema() {
    fn schema() -> Schema<BTreeMap<String, Unknown>> {
        Schema::struct_shape(vec![
            Field::required("a", str_schema()),
            Field::required(
                "as",
                Schema::as_unknown(Schema::array(Schema::as_unknown(
                    Schema::<Unknown>::suspend(|| Schema::as_unknown(schema())),
                ))),
            ),
        ])
    }
    let equivalence = Schema::as_unknown(schema()).to_equivalence();
    let empty = obj(vec![("a", text("a")), ("as", Unknown::Array(vec![]))]);
    let nested = obj(vec![
        ("a", text("a")),
        ("as", Unknown::Array(vec![empty.clone()])),
    ]);
    assert!(equivalence(&empty, &empty.clone()));
    assert!(!equivalence(
        &empty,
        &obj(vec![("a", text("b")), ("as", Unknown::Array(vec![]))])
    ));
    assert!(!equivalence(&nested, &empty));
    assert!(!equivalence(&empty, &nested));
}

// toEquivalence.test.ts:339 — mutually recursive schemas: the relation reaches
// through a union member that is suspended back to the schema being compared.
#[test]
fn suspend_compares_mutually_recursive_schemas() {
    fn expression() -> Schema<BTreeMap<String, Unknown>> {
        Schema::struct_shape(vec![
            Field::required("type", Schema::literal(text("expression"))),
            Field::required(
                "value",
                Schema::union_with_mode(
                    vec![
                        Schema::as_unknown(Schema::number()),
                        Schema::<Unknown>::suspend(|| Schema::as_unknown(operation())),
                    ],
                    "anyOf",
                ),
            ),
        ])
    }
    fn operation() -> Schema<BTreeMap<String, Unknown>> {
        Schema::struct_shape(vec![
            Field::required("type", Schema::literal(text("operation"))),
            Field::required("operator", Schema::literals(vec![text("+"), text("-")])),
            Field::required("left", Schema::as_unknown(expression())),
            Field::required("right", Schema::as_unknown(expression())),
        ])
    }
    fn expr(value: f64) -> Unknown {
        obj(vec![("type", text("expression")), ("value", num(value))])
    }
    fn op(operator: &str, left: Unknown, right: Unknown) -> Unknown {
        obj(vec![
            ("type", text("operation")),
            ("operator", text(operator)),
            ("left", left),
            ("right", right),
        ])
    }

    let equivalence = Schema::as_unknown(operation()).to_equivalence();
    let value = op("+", expr(1.0), expr(2.0));
    assert!(equivalence(&value, &op("+", expr(1.0), expr(2.0))));
    assert!(!equivalence(&value, &op("+", expr(1.0), expr(3.0))));
    assert!(!equivalence(&value, &op("-", expr(1.0), expr(2.0))));
    assert!(!equivalence(&value, &op("+", expr(2.0), expr(2.0))));
}

// toEquivalence.test.ts:421 — a Union member's relation is resolved once, and
// the member that owns these values decides them.
#[test]
fn union_precompiles_member_relations() {
    let derivations = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&derivations);
    let member = Schema::override_to_equivalence(
        Schema::as_unknown(Schema::struct_of(vec![
            ("tag".to_string(), Schema::literal(text("a"))),
            ("value".to_string(), str_schema()),
        ])),
        move || {
            counter.fetch_add(1, Ordering::SeqCst);
            Arc::new(|a: &Unknown, b: &Unknown| field(a, "value") == field(b, "value"))
        },
    );
    let equivalence =
        Schema::union_with_mode(vec![member, Schema::never()], "anyOf").to_equivalence();

    assert_eq!(derivations.load(Ordering::SeqCst), 1);
    let value = obj(vec![("tag", text("a")), ("value", text("a"))]);
    assert!(equivalence(
        &value,
        &obj(vec![("tag", text("a")), ("value", text("a"))])
    ));
    assert!(!equivalence(
        &value,
        &obj(vec![("tag", text("a")), ("value", text("b"))])
    ));
    assert_eq!(derivations.load(Ordering::SeqCst), 1);
}

/// A field of an `Unknown` object, for a member relation written on the typed
/// side the re-typed schema speaks.
fn field(value: &Unknown, name: &str) -> Unknown {
    match value {
        Unknown::Object(map) => map.get(name).cloned().unwrap_or(Unknown::Null),
        _ => Unknown::Null,
    }
}

// toEquivalence.test.ts:456 — a member's own relation survives the check it is
// wrapped in: the union must ask the member, not compare wire forms.
#[test]
fn union_preserves_member_relation_annotations() {
    let member = Schema::override_to_equivalence(
        Schema::checked(str_schema(), "isAlwaysTrue", Vec::new(), Vec::new()),
        || Arc::new(|a: &Unknown, b: &Unknown| first_char(a) == first_char(b)),
    );
    let equivalence =
        Schema::union_with_mode(vec![member, Schema::as_unknown(Schema::number())], "anyOf")
            .to_equivalence();

    assert!(equivalence(&text("ab"), &text("ac")));
    assert!(!equivalence(&text("ab"), &text("bc")));
}

fn first_char(value: &Unknown) -> Option<char> {
    match value {
        Unknown::Text(text) => text.chars().next(),
        _ => None,
    }
}

// toEquivalence.test.ts:469 — `Schema.Date` compares by instant.
#[test]
fn date_compares_by_millis() {
    let equivalence = date_schema().to_equivalence();
    assert!(equivalence(&0, &0));
    assert!(!equivalence(&0, &1));
}

// toEquivalence.test.ts:499 — `Option(A)` uses A's relation on the value side
// and its tag on the None side.
#[test]
fn option_uses_the_value_relation() {
    fn modulo2() -> Arc<dyn Fn(&f64, &f64) -> bool + Send + Sync> {
        Arc::new(|a: &f64, b: &f64| a % 2.0 == b % 2.0)
    }
    let equivalence = Schema::optional(number_with(modulo2)).to_equivalence();
    let none: Option<f64> = None;
    assert!(equivalence(&none, &none));
    assert!(equivalence(&Some(0.0), &Some(2.0)));
    assert!(equivalence(&Some(1.0), &Some(3.0)));
    assert!(!equivalence(&none, &Some(0.0)));
    assert!(!equivalence(&Some(0.0), &none));
    assert!(!equivalence(&Some(0.0), &Some(1.0)));
}

// toEquivalence.test.ts:647
#[test]
fn override_replaces_the_string_relation() {
    let equivalence = Schema::override_to_equivalence(Schema::string(), || {
        Arc::new(|a: &String, b: &String| a.chars().next() == b.chars().next())
    })
    .to_equivalence();
    assert!(equivalence(&"ab".to_string(), &"ac".to_string()));
    assert!(!equivalence(&"ab".to_string(), &"bc".to_string()));
}

// toEquivalence.test.ts:655 — the override wins over a check that wraps it.
#[test]
fn override_survives_a_check() {
    let checked = Schema::checked(str_schema(), "isMinLength", vec!["1"], Vec::new());
    let equivalence = Schema::override_to_equivalence(checked, || {
        Arc::new(|a: &Unknown, b: &Unknown| first_char(a) == first_char(b))
    })
    .to_equivalence();
    assert!(equivalence(&text("ab"), &text("ac")));
    assert!(!equivalence(&text("ab"), &text("bc")));
}

// toEquivalence.test.ts:663 — an override replaces a compiler-owned relation
// (`Option`'s) outright, so even `None` and `Some` compare equal.
#[test]
fn override_replaces_a_declaration_relation() {
    let equivalence = Schema::override_to_equivalence(Schema::optional(Schema::number()), || {
        Arc::new(|_: &Option<f64>, _: &Option<f64>| true)
    })
    .to_equivalence();
    assert!(equivalence(&None, &Some(1.0)));
}
