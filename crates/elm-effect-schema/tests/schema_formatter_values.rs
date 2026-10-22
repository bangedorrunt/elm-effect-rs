//! The `toFormatter` rows whose values the pin can only hold in a box: an
//! `Option`, a `Result`, a `Redacted`.
//!
//! The port's formatter takes the schema's wire value (`Unknown`), so each test
//! builds the wire value through the same schema's `try_encode` and then
//! formats it. That keeps the claim end to end: what the schema writes out is
//! what the formatter renders, and the rendering is the pin's.

use elm_effect_schema::{Schema, Unknown};

fn wire<T: Send + Sync + 'static>(schema: &Schema<T>, value: &T) -> Unknown
where
    T: std::fmt::Debug,
{
    schema.try_encode(value).expect("the value encodes")
}

/// toFormatter.test.ts:557 `Option(String)`.
#[test]
fn formatter_renders_a_boxed_option() {
    let schema = Schema::option(Schema::string());
    let format = schema.formatter();
    assert_eq!(
        format(&wire(&schema, &Some("a".to_string()))),
        "some(\"a\")"
    );
    assert_eq!(format(&wire(&schema, &None::<String>)), "none()");
}

/// toFormatter.test.ts:563 `Result(Number, String)`.
#[test]
fn formatter_renders_a_boxed_result() {
    let schema = Schema::result(Schema::number(), Schema::string());
    let format = schema.formatter();
    assert_eq!(format(&wire(&schema, &Ok::<f64, String>(1.0))), "success(1)");
    assert_eq!(
        format(&wire(&schema, &Err::<f64, String>("a".to_string()))),
        "failure(\"a\")"
    );
}

/// toFormatter.test.ts:582 `Redacted(String)`.
#[test]
fn formatter_renders_a_redacted_box() {
    let schema = Schema::redacted(Schema::as_unknown(Schema::string()), None);
    let format = schema.formatter();
    let boxed = wire(&schema, &Unknown::Object(
        [
            ("value".to_string(), Unknown::Text("a".to_string())),
        ]
        .into_iter()
        .collect(),
    ));
    assert_eq!(format(&boxed), "<redacted>");
}

/// toFormatter.test.ts:587 `with label`.
#[test]
fn formatter_renders_a_redacted_box_with_its_label() {
    let schema = Schema::redacted(Schema::as_unknown(Schema::string()), Some("password"));
    let format = schema.formatter();
    let boxed = wire(
        &schema,
        &Unknown::Object(
            [
                ("value".to_string(), Unknown::Text("a".to_string())),
                (
                    "label".to_string(),
                    Unknown::Text("password".to_string()),
                ),
            ]
            .into_iter()
            .collect(),
        ),
    );
    assert_eq!(format(&boxed), "<redacted:password>");
}

/// toFormatter.test.ts:225 `Array`.
#[test]
fn formatter_renders_an_array_of_options() {
    let schema = Schema::array(Schema::option(Schema::string()));
    let format = schema.formatter();
    let value = vec![Some("a".to_string())];
    assert_eq!(format(&wire(&schema, &value)), "[some(\"a\")]");
    let none: Vec<Option<String>> = vec![None];
    assert_eq!(format(&wire(&schema, &none)), "[none()]");
}

/// toFormatter.test.ts:218 `elements`.
#[test]
fn formatter_renders_a_tuple_of_options() {
    let schema = Schema::tuple_of(vec![Schema::as_unknown(Schema::option(
        Schema::string(),
    ))]);
    let format = schema.formatter();
    let some = Unknown::Object(
        [
            ("_tag".to_string(), Unknown::Text("Some".to_string())),
            ("value".to_string(), Unknown::Text("a".to_string())),
        ]
        .into_iter()
        .collect(),
    );
    let none = Unknown::Object(
        [("_tag".to_string(), Unknown::Text("None".to_string()))]
            .into_iter()
            .collect(),
    );
    assert_eq!(
        format(&Unknown::Array(vec![some])),
        "[some(\"a\")]"
    );
    assert_eq!(format(&Unknown::Array(vec![none])), "[none()]");
}

/// toFormatter.test.ts:290 `required fields` and :307 `optionalKey field`.
#[test]
fn formatter_renders_struct_fields_that_hold_options() {
    let required = Schema::struct_of(vec![(
        "a".to_string(),
        Schema::as_unknown(Schema::option(Schema::string())),
    )]);
    let format = required.formatter();
    let some = Unknown::Object(
        [(
            "a".to_string(),
            Unknown::Object(
                [
                    ("_tag".to_string(), Unknown::Text("Some".to_string())),
                    ("value".to_string(), Unknown::Text("a".to_string())),
                ]
                .into_iter()
                .collect(),
            ),
        )]
        .into_iter()
        .collect(),
    );
    assert_eq!(format(&some), "{ \"a\": some(\"a\") }");

    let optional_key = Schema::struct_of(vec![(
        "a".to_string(),
        Schema::optional_key_shape(Schema::as_unknown(Schema::option(Schema::string()))),
    )]);
    let format = optional_key.formatter();
    let boxed = |tag: &str, value: Option<&str>| {
        let mut fields = vec![(
            "_tag".to_string(),
            Unknown::Text(tag.to_string()),
        )];
        if let Some(value) = value {
            fields.push(("value".to_string(), Unknown::Text(value.to_string())));
        }
        Unknown::Object(fields.into_iter().collect())
    };
    let struct_value = |item: Unknown| {
        Unknown::Object([("a".to_string(), item)].into_iter().collect())
    };
    assert_eq!(
        format(&struct_value(boxed("Some", Some("a")))),
        "{ \"a\": some(\"a\") }"
    );
    assert_eq!(
        format(&struct_value(boxed("None", None))),
        "{ \"a\": none() }"
    );
    assert_eq!(format(&Unknown::Object(Default::default())), "{}");
}

/// toFormatter.test.ts:316 `optional field`.
///
/// The difference from :307 is what an absent key means: `optionalKey` drops
/// the key, `optional` admits `undefined` as a value, so a key that IS there
/// with `undefined` in it prints. The port spells the value domain as
/// `UndefinedOr(Option(...))`, which is the pin's `Option<string> | undefined`.
#[test]
fn formatter_renders_a_present_undefined_on_an_optional_field() {
    let field = Schema::as_unknown(Schema::undefined_or(Schema::as_unknown(Schema::option(
        Schema::string(),
    ))));
    let schema = Schema::struct_shape(vec![elm_effect_schema::Field::optional("a", field)]);
    let format = schema.formatter();
    let entry = |value: Unknown| {
        Unknown::Object([("a".to_string(), value)].into_iter().collect())
    };
    let some = Unknown::Object(
        [
            ("_tag".to_string(), Unknown::Text("Some".to_string())),
            ("value".to_string(), Unknown::Text("a".to_string())),
        ]
        .into_iter()
        .collect(),
    );
    let none = Unknown::Object(
        [("_tag".to_string(), Unknown::Text("None".to_string()))]
            .into_iter()
            .collect(),
    );
    assert_eq!(format(&entry(some)), "{ \"a\": some(\"a\") }");
    assert_eq!(format(&entry(none)), "{ \"a\": none() }");
    assert_eq!(
        format(&entry(Unknown::Undefined)),
        "{ \"a\": undefined }"
    );
    assert_eq!(format(&Unknown::Object(Default::default())), "{}");
}

/// toFormatter.test.ts:328 `Record(String, Option(Number))`.
#[test]
fn formatter_renders_a_record_of_options() {
    let schema = Schema::record(Schema::as_unknown(Schema::option(Schema::number())));
    let format = schema.formatter();
    let entry = |tag: &str, value: Option<f64>| {
        let mut fields = vec![(
            "_tag".to_string(),
            Unknown::Text(tag.to_string()),
        )];
        if let Some(value) = value {
            fields.push(("value".to_string(), Unknown::Number(value)));
        }
        Unknown::Object(fields.into_iter().collect())
    };
    let record = |item: Unknown| {
        Unknown::Object([("a".to_string(), item)].into_iter().collect())
    };
    assert_eq!(format(&record(entry("Some", Some(1.0)))), "{ \"a\": some(1) }");
    assert_eq!(format(&record(entry("None", None))), "{ \"a\": none() }");
}

/// toFormatter.test.ts:334 `Record(String.check, Option(Number)) should use the
/// key checks to select keys`.
#[test]
fn formatter_renders_only_the_record_keys_its_key_check_selects() {
    let key = Schema::checked(
        Schema::as_unknown(Schema::string()),
        "isPattern",
        vec!["new RegExp(\"^a\")"],
        Vec::new(),
    );
    let schema = Schema::record2(key, Schema::as_unknown(Schema::option(Schema::number())));
    let format = schema.formatter();
    let option_wire = |value: f64| {
        Unknown::Object(
            [
                ("_tag".to_string(), Unknown::Text("Some".to_string())),
                ("value".to_string(), Unknown::Number(value)),
            ]
            .into_iter()
            .collect(),
        )
    };
    let value = Unknown::Object(
        [
            ("a".to_string(), option_wire(1.0)),
            ("b".to_string(), option_wire(2.0)),
        ]
        .into_iter()
        .collect(),
    );
    assert_eq!(format(&value), "{ \"a\": some(1) }");
}

/// toFormatter.test.ts:510 `ReadonlySet`, recursive.
#[test]
fn formatter_renders_a_readonly_set() {
    fn set() -> Schema<Unknown> {
        Schema::as_unknown(Schema::<Vec<Unknown>>::readonly_set(Schema::<Unknown>::suspend(
            set,
        )))
    }
    let format = set().formatter();
    assert_eq!(format(&Unknown::Array(Vec::new())), "ReadonlySet(0) {}");
    let nested = Unknown::Array(vec![Unknown::Array(vec![Unknown::Array(Vec::new())])]);
    assert_eq!(
        format(&nested),
        "ReadonlySet(1) { ReadonlySet(1) { ReadonlySet(0) {} } }"
    );
}

/// toFormatter.test.ts:518 `ReadonlyMap`, recursive.
#[test]
fn formatter_renders_a_readonly_map() {
    fn map() -> Schema<Unknown> {
        Schema::as_unknown(Schema::<Vec<(Unknown, Unknown)>>::readonly_map(
            Schema::as_unknown(Schema::string()),
            Schema::<Unknown>::suspend(map),
        ))
    }
    let format = map().formatter();
    assert_eq!(format(&Unknown::Array(Vec::new())), "ReadonlyMap(0) {}");
    let pair = |key: &str, value: Unknown| {
        Unknown::Array(vec![Unknown::Text(key.to_string()), value])
    };
    let nested = Unknown::Array(vec![pair("a", Unknown::Array(vec![pair("b", Unknown::Array(Vec::new()))]))]);
    assert_eq!(
        format(&nested),
        "ReadonlyMap(1) { \"a\" => ReadonlyMap(1) { \"b\" => ReadonlyMap(0) {} } }"
    );
}

/// toFormatter.test.ts:529 `HashMap`.
#[test]
fn formatter_renders_a_hash_map() {
    let schema = Schema::hash_map(
        Schema::as_unknown(Schema::string()),
        Schema::as_unknown(Schema::number()),
    );
    let format = schema.formatter();
    assert_eq!(format(&Unknown::Array(Vec::new())), "HashMap(0) {}");
    let entry = Unknown::Array(vec![Unknown::Array(vec![
        Unknown::Text("a".to_string()),
        Unknown::Number(1.0),
    ])]);
    assert_eq!(format(&entry), "HashMap(1) { \"a\" => 1 }");
}

/// toFormatter.test.ts:569 `ReadonlyMap(String, Option(Number))`.
#[test]
fn formatter_renders_a_readonly_map_of_options() {
    let schema = Schema::readonly_map(
        Schema::as_unknown(Schema::string()),
        Schema::as_unknown(Schema::option(Schema::number())),
    );
    let format = schema.formatter();
    let entry = |tag: &str, value: Option<f64>| {
        let mut fields = vec![("_tag".to_string(), Unknown::Text(tag.to_string()))];
        if let Some(value) = value {
            fields.push(("value".to_string(), Unknown::Number(value)));
        }
        Unknown::Array(vec![
            Unknown::Text("a".to_string()),
            Unknown::Object(fields.into_iter().collect()),
        ])
    };
    assert_eq!(
        format(&Unknown::Array(vec![entry("Some", Some(1.0))])),
        "ReadonlyMap(1) { \"a\" => some(1) }"
    );
    assert_eq!(
        format(&Unknown::Array(vec![entry("None", None)])),
        "ReadonlyMap(1) { \"a\" => none() }"
    );
}

/// toFormatter.test.ts:575 `HashMap(String, Option(Number))`.
#[test]
fn formatter_renders_a_hash_map_of_options() {
    let schema = Schema::hash_map(
        Schema::as_unknown(Schema::string()),
        Schema::as_unknown(Schema::option(Schema::number())),
    );
    let format = schema.formatter();
    let entry = Unknown::Array(vec![
        Unknown::Text("a".to_string()),
        Unknown::Object(
            [
                ("_tag".to_string(), Unknown::Text("Some".to_string())),
                ("value".to_string(), Unknown::Number(1.0)),
            ]
            .into_iter()
            .collect(),
        ),
    ]);
    assert_eq!(
        format(&Unknown::Array(vec![entry])),
        "HashMap(1) { \"a\" => some(1) }"
    );
}

/// toFormatter.test.ts:298 `required field with undefined`.
#[test]
fn formatter_renders_an_option_of_undefined() {
    let field = Schema::as_unknown(Schema::option(Schema::undefined_or(Schema::string())));
    let schema = Schema::struct_of(vec![("a".to_string(), field)]);
    let format = schema.formatter();
    let boxed = |tag: &str, value: Option<Unknown>| {
        let mut fields = vec![("_tag".to_string(), Unknown::Text(tag.to_string()))];
        if let Some(value) = value {
            fields.push(("value".to_string(), value));
        }
        Unknown::Object([("a".to_string(), Unknown::Object(fields.into_iter().collect()))]
            .into_iter()
            .collect())
    };
    assert_eq!(
        format(&boxed("Some", Some(Unknown::Text("a".to_string())))),
        "{ \"a\": some(\"a\") }"
    );
    assert_eq!(
        format(&boxed("Some", Some(Unknown::Undefined))),
        "{ \"a\": some(undefined) }"
    );
    assert_eq!(format(&boxed("None", None)), "{ \"a\": none() }");
}

/// toFormatter.test.ts:497 `Option` with a recursive struct.
#[test]
fn formatter_renders_a_recursive_option() {
    // `Schema.Struct({ a: String, as: Option(Rec) })` where `Rec` is the struct
    // itself: rendering terminates on the value's depth, not on the schema's.
    fn recursive() -> Schema<Unknown> {
        Schema::<Unknown>::suspend(|| {
            Schema::as_unknown(Schema::struct_of(vec![
                ("a".to_string(), Schema::as_unknown(Schema::string())),
                (
                    "as".to_string(),
                    Schema::as_unknown(Schema::option(recursive())),
                ),
            ]))
        })
    }
    let format = recursive().formatter();
    let boxed = |value: Option<Unknown>| {
        let mut fields = vec![(
            "_tag".to_string(),
            Unknown::Text(match value {
                Some(_) => "Some".to_string(),
                None => "None".to_string(),
            }),
        )];
        if let Some(value) = value {
            fields.push(("value".to_string(), value));
        }
        Unknown::Object(fields.into_iter().collect())
    };
    let value = Unknown::Object(
        [
            ("a".to_string(), Unknown::Text("a".to_string())),
            (
                "as".to_string(),
                boxed(Some(Unknown::Object(
                    [
                        ("a".to_string(), Unknown::Text("b".to_string())),
                        ("as".to_string(), boxed(None)),
                    ]
                    .into_iter()
                    .collect(),
                ))),
            ),
        ]
        .into_iter()
        .collect(),
    );
    assert_eq!(
        format(&value),
        "{ \"a\": \"a\", \"as\": some({ \"a\": \"b\", \"as\": none() }) }"
    );
}
