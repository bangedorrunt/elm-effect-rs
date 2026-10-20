//! `Schema.test.ts` class-construction rows that do not need an object model:
//! a class over a refined source, and where a constructor default is allowed to
//! run.
//!
//! Four rows in the same cluster stay open and are excused in the ledger for
//! one reason: this port's decoded values are JSON (`Unknown`), so a class
//! instance cannot live inside a struct or union value. `:6869` and `:6884`
//! assert that a nested class value comes back as the instance, `:7047` asserts
//! `instanceof` inside struct and union values, and `:7056` asserts the union
//! construction selects an instance. None of those are expressible while
//! `BTreeMap<String, Unknown>` is the only struct value.
//!
//!     cargo test -p elm-effect-schema --test schema_class_make

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use elm_effect_schema::class::{class, class_from};
use elm_effect_schema::{Field, FilterOutcome, FilterSpec, Schema, Unknown};

fn text(value: &str) -> Unknown {
    Unknown::Text(value.to_string())
}

fn obj(pairs: Vec<(&str, Unknown)>) -> Unknown {
    Unknown::Object(
        pairs
            .into_iter()
            .map(|(key, value)| (key.to_string(), value))
            .collect(),
    )
}

fn map(pairs: Vec<(&str, Unknown)>) -> BTreeMap<String, Unknown> {
    pairs
        .into_iter()
        .map(|(key, value)| (key.to_string(), value))
        .collect()
}

fn string() -> Schema<Unknown> {
    Schema::as_unknown(Schema::string())
}

fn number() -> Schema<Unknown> {
    Schema::as_unknown(Schema::number())
}

fn struct_shape(fields: Vec<Field>) -> Schema<BTreeMap<String, Unknown>> {
    Schema::struct_shape(fields)
}

/// A filter that answers pass and counts how often it ran, which is how the
/// rows measure "validates once".
fn counting(counter: Arc<AtomicUsize>) -> FilterSpec {
    FilterSpec::new(move |_value| {
        counter.fetch_add(1, Ordering::SeqCst);
        FilterOutcome::Pass
    })
}

/// `Schema.test.ts:6853` — "decoding validates the class struct once": a class
/// over a refined struct keeps that struct as its source, so the check runs
/// once per decode rather than once per field or not at all.
#[test]
fn a_class_over_a_checked_source_validates_it_once() {
    let checks = Arc::new(AtomicUsize::new(0));
    let source = Schema::<Unknown>::filtered(
        Schema::as_unknown(struct_shape(vec![Field::required("a", string())])),
        vec![counting(Arc::clone(&checks))],
        false,
    );
    let class = class_from("A", source);

    let instance = class
        .decode(obj(vec![("a", text("a"))]))
        .expect("decodes into an instance");

    assert_eq!(instance.identifier(), "A");
    assert_eq!(
        checks.load(Ordering::SeqCst),
        1,
        "the source check runs once"
    );

    // The declared fields came through the refinement, so the class's excess
    // policy keeps `a` instead of stripping everything.
    assert_eq!(class.class_fields(), vec!["a".to_string()]);
    assert_eq!(
        instance.to_unknown(),
        obj(vec![("a", text("a"))]),
        "the instance carries the field the source kept"
    );
}

/// `Schema.test.ts:6981` — "Equality": an instance's identity is its values, so
/// two separately built instances with equal fields are equal. The pin reaches
/// that claim through immer drafts, which have no analogue here; building the
/// same instance twice is the port's spelling of it.
#[test]
fn class_instances_compare_by_their_values() {
    let a = class("A", vec![("a", string())]);
    let first = a.decode(obj(vec![("a", text("a1"))])).expect("decodes");
    let second = a.decode(obj(vec![("a", text("a1"))])).expect("decodes");
    let other = a.decode(obj(vec![("a", text("a"))])).expect("decodes");

    assert_eq!(first, second);
    assert_ne!(first, other);
}

/// `Schema.test.ts:6998` — "mapFields": a class's field list can be extended,
/// and the rebuilt class decodes the added field by its own schema.
#[test]
fn map_fields_rebuilds_a_class_with_the_mapped_list() {
    let a = class("A", vec![("a", string())]);
    assert_eq!(a.class_fields(), vec!["a".to_string()]);

    let schema = a.map_fields(|mut fields| {
        fields.push(Field::required("b", number()));
        fields
    });

    assert_eq!(
        schema.class_fields(),
        vec!["a".to_string(), "b".to_string()]
    );

    let instance = schema
        .decode(obj(vec![("a", text("a")), ("b", Unknown::Number(2.0))]))
        .expect("decodes into an instance");
    assert_eq!(instance.identifier(), "A", "the class keeps its identity");
    assert_eq!(
        instance.to_unknown(),
        obj(vec![("a", text("a")), ("b", Unknown::Number(2.0))])
    );
    assert!(
        schema
            .decode(obj(vec![("a", text("a")), ("b", text("x"))]))
            .is_err(),
        "the added field is decoded by its own schema"
    );
}

/// `Schema.test.ts:6912` — "make applies constructor defaults only at a field
/// occurrence": not at a bare construction, and not through a union member.
#[test]
fn a_constructor_default_runs_only_at_a_field_occurrence() {
    let defaults = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&defaults);
    let defaulted = Schema::as_unknown(Schema::<String>::string().with_constructor_default(
        move || {
            counter.fetch_add(1, Ordering::SeqCst);
            Ok(text("default"))
        },
    ));

    // A bare construction of an absent value answers nothing and runs nothing.
    assert!(
        defaulted
            .make_option(Unknown::Null)
            .expect("an absent value is not an issue")
            .is_none()
    );
    assert_eq!(defaults.load(Ordering::SeqCst), 0);

    // At a field occurrence the default fills the key.
    let field = struct_shape(vec![Field::required("value", defaulted.clone())]);
    assert_eq!(
        field.make(map(vec![])).expect("constructs"),
        map(vec![("value", text("default"))])
    );
    assert_eq!(defaults.load(Ordering::SeqCst), 1);

    // A union is not a field occurrence, so its member's default does not fill
    // the enclosing key: an absent or null value constructs nothing.
    let union_member = struct_shape(vec![Field::required(
        "value",
        Schema::as_unknown(Schema::union_with_mode(
            vec![defaulted.clone(), number()],
            "anyOf",
        )),
    )]);
    assert!(
        union_member
            .make_option(Unknown::Object(map(vec![])))
            .expect("an absent key is not an issue")
            .is_none(),
        "an absent key inside a union does not take a member's default"
    );
    assert!(
        union_member
            .make_option(Unknown::Object(map(vec![("value", Unknown::Null)])))
            .expect("a null key is not an issue")
            .is_none(),
        "a null key does not either"
    );
    assert_eq!(
        defaults.load(Ordering::SeqCst),
        1,
        "the default ran exactly once, at the field occurrence"
    );
}
