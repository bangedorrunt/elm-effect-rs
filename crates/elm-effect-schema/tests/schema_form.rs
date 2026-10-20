//! `Schema.fromURLSearchParams` and `Schema.fromFormData`: bracket-notated
//! pairs in, a nested value out, and the same pairs on the way back.
//!
//! The pin's wire is a `URLSearchParams`/`FormData` object; this port's wire is
//! the encoded text both of them represent (`a=a&b[c]=bc&e=e0&e=e1`), because
//! `Unknown` has no such object. The pairs, the nesting and the array
//! spellings are what the rows are about.
//!
//!     cargo test -p elm-effect-schema --test schema_form

use std::collections::BTreeMap;

use elm_effect_schema::checks::is_min_length;
use elm_effect_schema::{Field, Schema, Unknown};

fn obj(pairs: Vec<(&str, Unknown)>) -> Unknown {
    Unknown::Object(
        pairs
            .into_iter()
            .map(|(key, value)| (key.to_string(), value))
            .collect::<BTreeMap<_, _>>(),
    )
}

fn text(value: &str) -> Unknown {
    Unknown::Text(value.to_string())
}

fn strings(values: &[&str]) -> Unknown {
    Unknown::Array(values.iter().map(|value| text(value)).collect())
}

fn at_least_one_char() -> Schema<Unknown> {
    Schema::filtered(
        Schema::as_unknown(Schema::string()),
        vec![is_min_length(1)],
        false,
    )
}

/// The struct both rows decode: `{ a: NonEmptyString, b: { c, d }, e: [string] }`.
fn form_struct() -> Schema<Unknown> {
    Schema::as_unknown(Schema::struct_shape(vec![
        Field::required("a", at_least_one_char()),
        Field::required(
            "b",
            Schema::as_unknown(Schema::struct_shape(vec![
                Field::required("c", Schema::as_unknown(Schema::string())),
                Field::required("d", Schema::as_unknown(Schema::string())),
            ])),
        ),
        Field::required("e", Schema::as_unknown(Schema::array(Schema::string()))),
    ]))
}

fn decoded() -> Unknown {
    obj(vec![
        ("a", text("a")),
        ("b", obj(vec![("c", text("bc")), ("d", text("bd"))])),
        ("e", strings(&["e0", "e1"])),
    ])
}

/// Schema.test.ts:6137 — "fromURLSearchParams": the pairs nest by bracket path
/// and a repeated key is one array.
#[test]
fn url_search_params_decode_and_encode_the_bracket_pairs() {
    let schema = Schema::from_url_search_params(form_struct());

    assert_eq!(
        schema
            .decode(text("a=a&b[c]=bc&b[d]=bd&e=e0&e=e1"))
            .expect("the pairs decode"),
        decoded()
    );
    assert_eq!(
        schema
            .try_encode(&decoded())
            .expect("the value encodes back to pairs"),
        text("a=a&b[c]=bc&b[d]=bd&e=e0&e=e1")
    );

    // A field the inner schema refuses reports at the field, which is where the
    // pair was.
    let error = schema.decode(text("a=")).unwrap_err().to_string();
    assert!(
        error.contains("length of at least 1") && error.contains("$.a"),
        "{error}"
    );
}

/// Schema.test.ts:6091 — "fromFormData": a form body posts the same pairs, and
/// its array spelling is the indexed one (`e[0]`), which decodes to the same
/// array the repeated-key spelling does.
#[test]
fn form_data_accepts_indexed_array_pairs_and_writes_repeated_keys() {
    let schema = Schema::from_form_data(form_struct());

    assert_eq!(
        schema
            .decode(text("a=a&b[c]=bc&b[d]=bd&e[0]=e0&e[1]=e1"))
            .expect("indexed array pairs decode"),
        decoded(),
        "the indexed spelling and the repeated-key spelling name one array"
    );
    assert_eq!(
        schema
            .try_encode(&decoded())
            .expect("the value encodes back to pairs"),
        text("a=a&b[c]=bc&b[d]=bd&e=e0&e=e1"),
        "a leaf array writes one pair per element"
    );

    // The pin's failing form: an empty value for a non-empty field.
    let error = schema.decode(text("a=")).unwrap_err().to_string();
    assert!(
        error.contains("length of at least 1") && error.contains("$.a"),
        "{error}"
    );
}
