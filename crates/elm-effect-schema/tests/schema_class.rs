//! The `Class` family (Schema.test.ts :6810-:7791, plus the `Error` and
//! `TaggedClass` describes): construction, identity and the tagged wire shape.
//!
//! The pin's classes are JS classes; this port's are named constructible
//! schemas over a field list, and an instance is a `ClassValue`. Claims about
//! the JS object model (`instanceof`, `immer`, static members, a memoized ast)
//! are not witnessed here; the ones about what a class *decides* are.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use elm_effect_schema::class::{
    ExcessProperty, class, error, error_named, tagged_class, tagged_error,
};
use elm_effect_schema::{Field, Schema, Unknown};

fn text(value: &str) -> Unknown {
    Unknown::Text(value.to_string())
}

fn number(value: f64) -> Unknown {
    Unknown::Number(value)
}

fn object(pairs: Vec<(&str, Unknown)>) -> Unknown {
    Unknown::Object(
        pairs
            .into_iter()
            .map(|(name, value)| (name.to_string(), value))
            .collect(),
    )
}

fn string_field() -> Schema<Unknown> {
    Schema::as_unknown(Schema::string())
}

fn number_field() -> Schema<Unknown> {
    Schema::as_unknown(Schema::number())
}

// ---------------------------------------------------------------------------
// Class: `make` and the constructor rules
// ---------------------------------------------------------------------------

#[test]
fn make_fills_constructor_defaults_on_construction() {
    // :6820 "make applies defaults, source checks, and the constructor once":
    // the two claims this port can observe are that the field's default fills
    // and that it is evaluated exactly once. The pin's "constructor once" is a
    // JS reference identity claim (`A.make(instance) === instance`), which the
    // port answers with `make_instance` below.
    let defaults = Arc::new(AtomicUsize::new(0));
    let defaulted = Schema::with_constructor_default(string_field(), {
        let defaults = Arc::clone(&defaults);
        move || {
            defaults.fetch_add(1, Ordering::SeqCst);
            Ok(text("default"))
        }
    });
    let schema = class("A", vec![("a", defaulted)]);
    let built = schema.make(Unknown::Object(BTreeMap::new())).unwrap();
    assert_eq!(built.get("a"), Some(&text("default")));
    assert_eq!(defaults.load(Ordering::SeqCst), 1, "the default fills once");
    assert!(built.is_instance_of("A"));
    // The construction path is not the decoding path: a decode of the same
    // input leaves the key out rather than filling it.
    assert!(schema.decode(Unknown::Object(BTreeMap::new())).is_err());
}

#[test]
fn make_with_void_input_builds_a_zero_field_class() {
    // :6846 "make with void input": `A.make()` for a class with no fields.
    let empty = class("A", vec![]);
    let built = empty.make_void().unwrap();
    assert_eq!(built.identifier(), "A");
    assert!(built.field_names().is_empty());
    assert_eq!(built.to_unknown(), Unknown::Object(BTreeMap::new()));
}

#[test]
fn a_tagged_class_takes_a_void_construction() {
    // :7395 "TaggedClass make with void input": the discriminant is the only
    // field and it carries the tag, so `make()` builds the instance from an
    // empty input. The pin's `makeEffect` is the same construction here, and
    // `makeOption` reports the same value as `Some`.
    let schema = tagged_class("A", vec![]);
    let built = schema.make_void().unwrap();
    assert!(built.is_instance_of("A"));
    assert_eq!(built.tag(), Some("A"));
    assert_eq!(built.to_unknown(), object(vec![("_tag", text("A"))]));

    let optional = schema
        .make_option(Unknown::Object(BTreeMap::new()))
        .unwrap();
    assert_eq!(optional, Some(built), "makeOption answers the same value");
}

#[test]
fn an_extended_constructor_strips_excess_under_explicit_ignore() {
    // :7322 "constructor keeps subclass fields and strips excess properties
    // with explicit ignore": the call site's excess policy is `ignore`, and the
    // subclass field is still declared rather than treated as excess.
    let child = class("A", vec![("a", string_field())])
        .extend("B", vec![("b", number_field())])
        .with_excess(ExcessProperty::Ignore);

    let built = child
        .make(object(vec![
            ("a", text("a")),
            ("b", number(2.0)),
            ("extra", text("extra")),
        ]))
        .unwrap();
    assert!(built.is_instance_of("B"));
    assert_eq!(built.get("a"), Some(&text("a")));
    assert_eq!(built.get("b"), Some(&number(2.0)));
    assert_eq!(built.get("extra"), None, "the extra key is stripped");
}

#[test]
fn make_keeps_class_instances_inside_arrays_and_unions() {
    // :7035 "make preserves Class instances in Array(Class) and
    // Array(Union(Class))": a constructed element is still the class's instance,
    // and an element the class refuses fails the construction instead of
    // arriving as a plain object.
    let row = class("Row", vec![("value", string_field())]);
    let row_value = row
        .make(object(vec![("value", text("a"))]))
        .expect("a row is constructible");
    let direct = class(
        "DirectTable",
        vec![(
            "rows",
            Schema::as_unknown(Schema::array(row.clone())),
        )],
    );
    let union = class(
        "UnionTable",
        vec![(
            "rows",
            Schema::as_unknown(Schema::array(Schema::union_with_mode(
                vec![Schema::as_unknown(row.clone())],
                "anyOf",
            ))),
        )],
    );

    for schema in [&direct, &union] {
        let built = schema
            .make(object(vec![(
                "rows",
                Unknown::Array(vec![row_value.to_unknown()]),
            )]))
            .expect("the table constructs");
        let rows = match built.get("rows") {
            Some(Unknown::Array(rows)) => rows.clone(),
            other => panic!("expected an array of rows, got {other:?}"),
        };
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0], row_value.to_unknown());

        let refused = schema
            .make(object(vec![(
                "rows",
                Unknown::Array(vec![object(vec![("value", number(1.0))])]),
            )]))
            .is_err();
        assert!(refused, "a row whose value is not text is refused");
    }
}

#[test]
fn make_answers_an_existing_instance_unchanged() {
    // :7656 "make preserves existing instances": the value comes back as it
    // was. The pin compares by reference; the identity here is the identifier
    // the value was built under, and another class's value is not this class's
    // to answer for.
    let schema = tagged_error("E", "E", vec![("id", number_field())]);
    let built = schema
        .make(object(vec![("_tag", text("E")), ("id", number(1.0))]))
        .unwrap();
    let again = schema.make_instance(&built).unwrap();
    assert_eq!(again, built);
    let other = tagged_error("F", "F", vec![("id", number_field())]);
    assert!(
        other.make_instance(&built).is_err(),
        "another class does not own the value"
    );
}

// ---------------------------------------------------------------------------
// Class: the constructor's excess-property policy
// ---------------------------------------------------------------------------

#[test]
fn the_constructor_ignores_excess_properties_by_default() {
    // :7195.
    let instance = class("A", vec![("a", string_field())])
        .make(object(vec![("a", text("a")), ("extra", text("extra"))]))
        .unwrap();
    assert_eq!(instance.get("a"), Some(&text("a")));
    assert_eq!(instance.get("extra"), None);
    assert_eq!(instance.field_names(), vec!["a"]);
}

#[test]
fn an_explicit_ignore_or_strip_still_leaves_the_key_off() {
    // :7206 "constructor strips excess properties with explicit ignore": the
    // two policies differ in how the key leaves the source object, and the
    // instance is the same either way.
    for policy in [ExcessProperty::Ignore, ExcessProperty::Strip] {
        let instance = class("A", vec![("a", string_field())])
            .with_excess(policy)
            .make(object(vec![("a", text("a")), ("extra", text("extra"))]))
            .unwrap();
        assert_eq!(
            instance.get("extra"),
            None,
            "{policy:?} keeps no excess key"
        );
    }
}

#[test]
fn the_constructor_rejects_excess_properties_when_requested() {
    // :7219 "constructor rejects excess properties when requested".
    let schema = class("A", vec![("a", string_field())]).with_excess(ExcessProperty::Reject);
    let refusal = schema
        .make(object(vec![("a", text("a")), ("extra", text("extra"))]))
        .unwrap_err()
        .to_string();
    assert!(
        refusal.contains("extra"),
        "the excess key is named: {refusal}"
    );
    assert!(schema.make(object(vec![("a", text("a"))])).is_ok());
}

// ---------------------------------------------------------------------------
// Class: the field list, the identifier, and the codec around it
// ---------------------------------------------------------------------------

#[test]
fn a_class_exposes_its_fields_and_identifier_and_round_trips() {
    // :7083 "Fields argument" and :7139 "Struct argument", asserted through the
    // port's vocabulary. The legs about a class body's extra fields (`_a = 1`)
    // and `instanceof` are the JS object model and have no witness here.
    let schema = class("A", vec![("a", string_field())]);
    assert_eq!(schema.class_identifier(), Some("A"));
    assert_eq!(schema.class_fields(), vec!["a"]);
    assert_eq!(
        schema.resolve_annotations(),
        Some(vec![("identifier".to_string(), "A".to_string())]),
        ":6953 the class carries an identifier annotation"
    );

    let decoded = schema.decode(object(vec![("a", text("a"))])).unwrap();
    assert_eq!(decoded.get("a"), Some(&text("a")));
    assert!(
        schema.decode(Unknown::Null).is_err(),
        "null is not an object"
    );
    let wrong_field = schema
        .decode(object(vec![("a", number(1.0))]))
        .unwrap_err()
        .to_string();
    assert!(
        wrong_field.contains("$.a"),
        "the field path is named: {wrong_field}"
    );

    assert_eq!(
        schema.encode_instance(&decoded).unwrap(),
        object(vec![("a", text("a"))])
    );
    // Encoding is instance-directed: another class's value is refused.
    let foreign = class("B", vec![("a", string_field())])
        .make(object(vec![("a", text("a"))]))
        .unwrap();
    let refusal = schema.encode_instance(&foreign).unwrap_err().to_string();
    assert!(
        refusal.contains("an instance of A"),
        "the refusal names the class: {refusal}"
    );
}

#[test]
fn a_class_make_reports_a_schema_issue_for_the_effect_channel() {
    // :3701 `makeEffect` `Class`: the accepted input constructs an instance and
    // the refused one answers the issue, which is this port's `Result`.
    let positive = number_field().refine(
        "greater than 0",
        |value: &Unknown| matches!(value, Unknown::Number(number) if *number > 0.0),
    );
    let schema = class("A", vec![("a", positive)]);

    let built = schema
        .make(object(vec![("a", number(1.0))]))
        .expect("a positive value is constructible");
    assert!(built.is_instance_of("A"));
    assert!(
        matches!(
            schema.make(object(vec![("a", number(-1.0))])),
            Err(elm_effect_schema::AdapterError::Issue(_))
        ),
        "a check's refusal is an issue"
    );
}

#[test]
fn an_opaque_struct_constructs_its_instance_and_keeps_its_fields() {
    // :5646 `Opaque` `Struct`: the branded class is the struct it was built
    // from, so `make` builds the instance and the declared fields are the
    // struct's own. `Opaque`'s type-level identity is the port's `class_from`
    // identifier.
    let schema = elm_effect_schema::class::class_from(
        "A",
        Schema::as_unknown(Schema::struct_shape(vec![Field::required("a", string_field())])),
    );

    let built = schema
        .make(object(vec![("a", text("a"))]))
        .expect("the struct is constructible");
    assert!(built.is_instance_of("A"));
    assert_eq!(built.get("a"), Some(&text("a")));
    assert_eq!(schema.class_fields(), vec!["a"], "the struct's own fields");
}

#[test]
fn a_class_make_option_answers_none_for_a_schema_issue() {
    // :3664 `makeOption` `Class`: the same rule at a class, where the accepted
    // input constructs an instance of the class.
    let positive = number_field().refine(
        "greater than 0",
        |value: &Unknown| matches!(value, Unknown::Number(number) if *number > 0.0),
    );
    let schema = class("A", vec![("a", positive)]);

    let built = schema
        .make_option(object(vec![("a", number(1.0))]))
        .unwrap()
        .expect("a positive value is constructible");
    assert!(built.is_instance_of("A"));
    assert_eq!(built.get("a"), Some(&number(1.0)));
    assert_eq!(
        schema.make_option(object(vec![("a", number(-1.0))])).unwrap(),
        None
    );
}

#[test]
fn an_annotation_over_a_class_keeps_the_class() {
    // :7231 "annotate": the annotated schema is still a schema with the same
    // fields, and its `make` still builds the class's value.
    let annotated = Schema::annotated_with(
        class("A", vec![("a", string_field())]),
        vec![("title", "annotated")],
    );
    assert_eq!(annotated.class_fields(), vec!["a"]);
    let built = annotated.make(object(vec![("a", text("a"))])).unwrap();
    assert!(built.is_instance_of("A"));
}

#[test]
fn a_nested_class_field_constructs_its_own_instance() {
    // :7006 "Struct with nested Class" / :7020 "Class with nested Class": the
    // nested field's schema is the class, so constructing the outer class
    // constructs the inner one.
    let inner = class("Inner", vec![("b", string_field())]);
    let outer = class("Outer", vec![("inner", Schema::as_unknown(inner.clone()))]);
    let built = outer
        .make(object(vec![("inner", object(vec![("b", text("b"))]))]))
        .unwrap();
    let nested = inner
        .decode(built.get("inner").cloned().expect("the nested field"))
        .unwrap();
    assert!(nested.is_instance_of("Inner"));
    assert_eq!(nested.get("b"), Some(&text("b")));
}

#[test]
fn an_optional_nested_class_may_be_omitted_from_a_construction() {
    // :6905 "make allows an optional nested Class to be omitted": an
    // `optionalKey` field whose schema is a class stays absent.
    let inner = class("Inner", vec![("b", string_field())]);
    let outer = Schema::struct_shape(vec![Field::optional_key(
        "inner",
        Schema::as_unknown(inner),
    )]);
    let built = outer.construct(object(vec![])).expect("may be absent");
    assert_eq!(built.get("inner"), None);
}

// ---------------------------------------------------------------------------
// TaggedClass / Error / TaggedError: the discriminant
// ---------------------------------------------------------------------------

#[test]
fn a_tagged_class_carries_its_tag_on_the_wire() {
    // :7409 "fields argument" (TaggedClass) and :7670 (TaggedError): the value
    // holds the fields, the wire holds them plus `_tag`, and the tag is
    // required on the way back in.
    let schema = tagged_class("E", vec![("id", number_field())]);
    assert_eq!(schema.class_tag(), Some("E"));
    let decoded = schema
        .decode(object(vec![("_tag", text("E")), ("id", number(1.0))]))
        .unwrap();
    assert_eq!(decoded.tag(), Some("E"));
    assert_eq!(decoded.get("id"), Some(&number(1.0)));
    assert_eq!(decoded.field_names(), vec!["id"], "the tag is not a field");
    assert_eq!(
        schema.encode_instance(&decoded).unwrap(),
        object(vec![("_tag", text("E")), ("id", number(1.0))])
    );

    let missing = schema
        .decode(object(vec![("id", number(1.0))]))
        .unwrap_err();
    assert!(missing.to_string().contains("_tag"), "the tag is required");
    let foreign = schema
        .decode(object(vec![("_tag", text("F")), ("id", number(1.0))]))
        .unwrap_err();
    assert!(foreign.to_string().contains("another tag"));
}

#[test]
fn a_tagged_classs_field_list_reports_the_discriminant() {
    // :7704 "Struct argument": `Object.keys(E.fields).sort()` is
    // ["_tag", "id"], which is the discriminant's place in the field map.
    let schema = tagged_error("E", "E", vec![("id", number_field())]);
    let mut names = schema.class_fields();
    names.sort();
    assert_eq!(names, vec!["_tag".to_string(), "id".to_string()]);
}

#[test]
fn the_name_is_the_identifier_and_the_tag_stands_in_when_there_is_none() {
    // :7718 "name matches tag": `TaggedError<E>()("TaggedErrorName", ...)` takes
    // its identifier from the tag.
    let same = tagged_error(
        "TaggedErrorName",
        "TaggedErrorName",
        vec![("id", number_field())],
    );
    assert_eq!(same.class_identifier(), Some("TaggedErrorName"));
    assert_eq!(same.class_tag(), Some("TaggedErrorName"));

    // :7727 "name matches identifier": `TaggedError<E>("A")("B", ...)` names the
    // identifier, while the wire carries the tag.
    let distinct = error_named("A", "B", vec![("a", number_field())]);
    assert_eq!(distinct.class_identifier(), Some("A"));
    assert_eq!(distinct.class_tag(), Some("B"));
    let built = distinct
        .make(object(vec![("_tag", text("B")), ("a", number(1.0))]))
        .unwrap();
    assert_eq!(built.identifier(), "A");
    assert_eq!(built.tag(), Some("B"));

    // `Schema.Error<E>()("E")(...)` defaults the identifier to the tag, which is
    // what the `Error` describe's `name matches tag` rows assert.
    let plain_error = error("E", vec![("message", string_field())]);
    assert_eq!(plain_error.class_identifier(), Some("E"));
    assert_eq!(plain_error.class_tag(), Some("E"));
}

#[test]
fn a_zero_field_tagged_error_accepts_an_omitted_props_argument() {
    // :7748 "zero-field TaggedError allows omitting props argument": both
    // `new E()` and `new E({})` work.
    let schema = tagged_error("NotFoundError", "NotFoundError", vec![]);
    let built = schema.make_void().unwrap();
    assert_eq!(built.tag(), Some("NotFoundError"));
    assert_eq!(
        built.to_unknown(),
        object(vec![("_tag", text("NotFoundError"))])
    );
    let explicit = schema
        .make(object(vec![("_tag", text("NotFoundError"))]))
        .unwrap();
    assert_eq!(explicit, built);
}

#[test]
fn a_tagged_error_ignores_excess_properties_by_default() {
    // :7692 "constructor ignores excess properties by default".
    let schema = tagged_error("E", "E", vec![("id", number_field())]);
    let instance = schema
        .make(object(vec![
            ("_tag", text("E")),
            ("id", number(1.0)),
            ("extra", text("x")),
        ]))
        .unwrap();
    assert_eq!(instance.get("id"), Some(&number(1.0)));
    assert_eq!(instance.get("extra"), None);
}

// ---------------------------------------------------------------------------
// extend
// ---------------------------------------------------------------------------

#[test]
fn extend_keeps_the_parents_tag_and_takes_the_childs_identifier() {
    // :7762 "extend": the extended instance reports the PARENT's tag, and the
    // child's identifier is what a name lookup answers with (:7736 "name matches
    // identifier after extend").
    let parent = tagged_error("A", "A", vec![("a", string_field())]);
    let child = parent.clone().extend("B", vec![("b", number_field())]);
    assert_eq!(child.class_identifier(), Some("B"));
    assert_eq!(child.class_tag(), Some("A"), "the tag stays the parent's");
    assert_eq!(child.class_fields(), vec!["_tag", "a", "b"]);

    let built = child
        .make(object(vec![
            ("_tag", text("A")),
            ("a", text("a")),
            ("b", number(2.0)),
        ]))
        .unwrap();
    assert!(built.is_instance_of("B"));
    assert!(!built.is_instance_of("A"), "identity is the child's");
    assert_eq!(built.tag(), Some("A"));
    assert_eq!(built.get("a"), Some(&text("a")));
    assert_eq!(built.get("b"), Some(&number(2.0)));
    // The parent still constructs its own shape.
    assert!(
        parent
            .make(object(vec![("_tag", text("A")), ("a", text("a"))]))
            .is_ok()
    );
}

#[test]
fn the_extended_constructor_ignores_excess_but_not_the_subclass_fields() {
    // :7775 "extended constructor ignores excess properties by default" and
    // :7791 "extended constructor does not treat subclass fields as excess
    // properties": with the reject policy the child's own fields are declared,
    // and only a genuine extra key fails.
    let child = tagged_error("A", "A", vec![("a", string_field())])
        .extend("B", vec![("b", number_field())])
        .with_excess(ExcessProperty::Reject);
    assert!(
        child
            .make(object(vec![
                ("_tag", text("A")),
                ("a", text("a")),
                ("b", number(2.0)),
            ]))
            .is_ok(),
        "the subclass field is declared, not excess"
    );
    let refusal = child
        .make(object(vec![
            ("_tag", text("A")),
            ("a", text("a")),
            ("b", number(2.0)),
            ("extra", text("x")),
        ]))
        .unwrap_err()
        .to_string();
    assert!(
        refusal.contains("extra"),
        "only the extra key is refused: {refusal}"
    );
}

#[test]
fn an_error_class_is_its_tagged_error_twin_and_prints_like_one() {
    // The `TaggedClass > Error` describe (:7503-:7646): `Schema.Error<E>("E")`
    // is the same builder, so the void-input make, the field list, the excess
    // policies, the extension and the `toString` shape all hold for it.
    let empty = error("E", vec![]);
    let built = empty.make_void().unwrap();
    assert_eq!(built.identifier(), "E");
    assert_eq!(built.tag(), Some("E"));

    let with_message = error("E", vec![("message", string_field())]);
    assert_eq!(with_message.class_fields(), vec!["_tag", "message"]);
    let value = with_message
        .make(object(vec![("message", text("my message"))]))
        .unwrap();
    // :7646 "toString to match native Error output format".
    assert_eq!(value.to_string(), "E: my message");
    // :7510 the wire carries the tag; :7527 an excess key is ignored.
    assert_eq!(
        with_message.encode_instance(&value).unwrap(),
        object(vec![("_tag", text("E")), ("message", text("my message"))])
    );
    let relaxed = with_message
        .make(object(vec![("message", text("m")), ("extra", text("x"))]))
        .unwrap();
    assert_eq!(relaxed.get("extra"), None);
    // :7543 an explicit ignore leaves it off too.
    let stripped = with_message
        .clone()
        .with_excess(ExcessProperty::Strip)
        .make(object(vec![("message", text("m")), ("extra", text("x"))]))
        .unwrap();
    assert_eq!(stripped.get("extra"), None);
    // :7579 the extension keeps the parent's tag; :7615/:7630 the child's own
    // fields are declared, so only a genuine extra key is excess.
    let child = with_message
        .clone()
        .extend("F", vec![("code", number_field())])
        .with_excess(ExcessProperty::Reject);
    assert_eq!(child.class_tag(), Some("E"));
    let extended = child
        .make(object(vec![("message", text("m")), ("code", number(1.0))]))
        .unwrap();
    assert_eq!(extended.get("code"), Some(&number(1.0)));
    assert_eq!(extended.to_string(), "F: m");
}

#[test]
fn an_untagged_class_extends_without_introducing_a_discriminant() {
    // The plain `Class` half of the extend rows: no `_tag` anywhere, and the
    // child's field list is the parent's plus its own.
    let child = class("A", vec![("a", string_field())]).extend("B", vec![("b", number_field())]);
    assert_eq!(child.class_tag(), None);
    assert_eq!(child.class_fields(), vec!["a", "b"]);
    let built = child
        .make(object(vec![("a", text("a")), ("b", number(2.0))]))
        .unwrap();
    assert_eq!(built.identifier(), "B");
    assert_eq!(
        built.to_unknown(),
        object(vec![("a", text("a")), ("b", number(2.0))])
    );
}
