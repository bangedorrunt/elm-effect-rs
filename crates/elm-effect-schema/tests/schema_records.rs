//! Record key schemas: which keys a record describes, and what an unmatched one
//! means. Derives from Schema.test.ts:4066-4300 and the `StructWithRest` row at
//! :5053.

use std::collections::BTreeMap;

use elm_effect_schema::class::ExcessProperty;
use elm_effect_schema::{Schema, TemplatePart, Unknown};

fn text(value: &str) -> Unknown {
    Unknown::Text(value.to_string())
}

fn obj(pairs: Vec<(&str, Unknown)>) -> Unknown {
    Unknown::Object(
        pairs
            .into_iter()
            .map(|(name, value)| (name.to_string(), value))
            .collect(),
    )
}

fn map(pairs: Vec<(&str, Unknown)>) -> BTreeMap<String, Unknown> {
    pairs
        .into_iter()
        .map(|(name, value)| (name.to_string(), value))
        .collect()
}

fn number_schema() -> Schema<Unknown> {
    Schema::as_unknown(Schema::number())
}

/// `Schema.String.check(Schema.isPattern(/^a/))`, the key schema the pin's
/// pattern rows use.
fn pattern_key(prefix: &str) -> Schema<Unknown> {
    Schema::checked(
        Schema::as_unknown(Schema::string()),
        "isPattern",
        vec![&format!("new RegExp(\"{prefix}\", \"u\")")],
        vec![],
    )
}

fn record_of(schema: Schema<BTreeMap<String, Unknown>>, value: Unknown) -> String {
    schema.decode(value).unwrap_err().to_string()
}

#[test]
fn record_excess_names_an_unmatched_key() {
    // :4066 "applies excess-property options to unmatched own keys": a key the
    // key schema refuses is excess, so `ignore` drops it and `error` names it.
    let input = obj(vec![
        ("a", Unknown::Number(1.0)),
        ("b", Unknown::Number(2.0)),
    ]);
    let expected = map(vec![("a", Unknown::Number(1.0))]);

    let ignoring = Schema::record2(pattern_key("^a"), number_schema());
    assert_eq!(ignoring.decode(input.clone()).unwrap(), expected);
    assert_eq!(
        ignoring
            .try_encode(&ignoring.decode(input.clone()).unwrap())
            .unwrap(),
        obj(vec![("a", Unknown::Number(1.0))])
    );

    let rejecting =
        Schema::record2_excess(pattern_key("^a"), number_schema(), ExcessProperty::Reject);
    let refusal = record_of(rejecting.clone(), input.clone());
    assert_eq!(refusal, "expected no excess property at $.b, got excess");
    // The policy also decides the encode side, so an unmatched key on the way
    // out is named rather than quietly written.
    let map: BTreeMap<String, Unknown> = match input {
        Unknown::Object(fields) => fields,
        other => panic!("an object: {other:?}"),
    };
    assert_eq!(
        rejecting.try_encode(&map).unwrap_err().to_string(),
        "expected no excess property at $.b, got excess"
    );
    // Every declared key still decodes.
    assert!(
        rejecting
            .decode(obj(vec![("a", Unknown::Number(1.0))]))
            .is_ok()
    );
}

/// A finite number on the wire, refused for `NaN` and both infinities, which is
/// what the pin's `Schema.Number` is.
fn finite_schema() -> Schema<Unknown> {
    Schema::checked(
        Schema::as_unknown(Schema::number()),
        "isFinite",
        vec![],
        vec![],
    )
}

#[test]
fn an_integer_key_schema_selects_the_numeric_names() {
    // :4270 "Record(Int, String)": a number key schema is offered the number a
    // name spells, so `"1"` is selected and `"1.1"`, `"Infinity"` and `"NaN"`
    // are unmatched keys rather than failures.
    //
    // The value schema is `FiniteFromString`, whose Unknown spelling in this
    // port is its wire text, so the selected entry reads `Text("1")` rather
    // than the pin's number. The two refusals below are what show the value
    // schema really ran instead of the key selection being the whole story.
    let schema = Schema::record2(
        Schema::as_unknown(Schema::int()),
        Schema::as_unknown(Schema::finite_from_string()),
    );
    assert_eq!(
        schema.decode(obj(vec![("1", text("1"))])).unwrap(),
        map(vec![("1", text("1"))])
    );
    assert_eq!(
        schema
            .decode(obj(vec![
                ("1", text("1")),
                ("1.1", text("ignored")),
                ("Infinity", text("ignored")),
                ("NaN", text("ignored")),
            ]))
            .unwrap(),
        map(vec![("1", text("1"))])
    );
    assert_eq!(
        record_of(schema.clone(), obj(vec![("1", Unknown::Null)])),
        "expected a finite number string at $.1, got null"
    );
    assert_eq!(
        record_of(schema, obj(vec![("1", text("a"))])),
        "expected a finite number string at $.1, got string"
    );
}

#[test]
fn a_template_literal_key_schema_selects_its_own_keys() {
    // :4289 `Record(TemplateLiteral(["a", NonEmptyString]), Number)`: the `a`
    // key alone does not match the template, so it is dropped; `ab` does.
    let schema = Schema::record2(
        Schema::as_unknown(Schema::template_literal(vec![
            TemplatePart::Anchor("a".to_string()),
            TemplatePart::NonEmptyString,
        ])),
        number_schema(),
    );
    assert_eq!(
        schema
            .decode(obj(vec![
                ("a", text("ignored")),
                ("ab", Unknown::Number(1.0))
            ]))
            .unwrap(),
        map(vec![("ab", Unknown::Number(1.0))])
    );
    assert_eq!(
        record_of(
            schema,
            obj(vec![("a", Unknown::Number(1.0)), ("ab", text("bad"))])
        ),
        "expected number at $.ab, got string"
    );
}

#[test]
fn a_record_rest_under_a_struct_selects_template_keys() {
    // :5053 `StructWithRest(Struct({ a: Number }), [Record(`a${string}`,
    // Number)])`: the fixed `a` decodes with its own schema, and the catch-all
    // takes the longer keys while a value of the wrong shape is named.
    let schema = Schema::struct_with_rest(
        vec![("a".to_string(), finite_schema())],
        Schema::as_unknown(Schema::record2(
            Schema::as_unknown(Schema::template_literal(vec![
                TemplatePart::Anchor("a".to_string()),
                TemplatePart::AnyString,
            ])),
            number_schema(),
        )),
    );
    assert!(
        schema
            .decode(obj(vec![("a", Unknown::Number(1.0))]))
            .is_ok()
    );
    let both = schema
        .decode(obj(vec![
            ("a", Unknown::Number(1.0)),
            ("ab", Unknown::Number(2.0)),
        ]))
        .unwrap();
    assert_eq!(
        both,
        obj(vec![
            ("a", Unknown::Number(1.0)),
            ("ab", Unknown::Number(2.0))
        ])
    );
    // The fixed property decodes only with its own schema, so a `NaN` there is
    // refused at `a` and not silently absorbed by the catch-all.
    let fixed = schema
        .decode(obj(vec![
            ("a", Unknown::Number(f64::NAN)),
            ("ab", Unknown::Number(2.0)),
        ]))
        .unwrap_err()
        .to_string();
    assert_eq!(
        fixed,
        "expected a value satisfying the refinement at $.a, got number"
    );
    let rest = schema
        .decode(obj(vec![("a", Unknown::Number(1.0)), ("ab", text("c"))]))
        .unwrap_err()
        .to_string();
    assert_eq!(rest, "expected number at $.ab, got string");
}
