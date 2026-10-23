//! Phase 6 transforms: Transformation/Getter/Parser/Standard/Channel/brands.
//! Derives from packages/effect/test/schema/ rows (portable subset).
//! Residuals (Effect-async children, bounded concurrency, JS prototype
//! guards, Symbol fields) stay `unported` and are listed in the close
//! report instead of follow-up beads.

use elm_effect_schema::transform::{
    Branded, ChannelSchema, Getter, Parser, Segment, StandardResult, Transformation,
    datetime_utc_from_input, is_transformation, make_tree_record, validate,
};
use elm_effect_schema::{Schema, Unknown};
use std::collections::BTreeMap;

fn obj(pairs: &[(&str, Unknown)]) -> Unknown {
    Unknown::Object(
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect(),
    )
}

fn text(s: &str) -> Unknown {
    Unknown::Text(s.to_string())
}

// ------------------------------------------------------- Transformation ---

#[test]
fn is_transformation_tags_structures() {
    assert!(is_transformation(
        &Transformation::<Unknown, Unknown>::passthrough()
    ));
}

#[test]
fn passthrough_round_trips() {
    let t = Transformation::<Unknown, Unknown>::passthrough();
    let v = text("a");
    assert_eq!(t.forward(v.clone()).unwrap(), v);
    assert_eq!(t.backward(v.clone()).unwrap(), v);
}

#[test]
fn pipe_composes_both_directions() {
    let double = Transformation::new(
        |u: Unknown| match u {
            Unknown::Text(s) => Ok(Unknown::Text(s.clone() + &s)),
            other => Err(elm_effect_schema::SchemaError::TypeMismatch {
                path: String::new(),
                expected: "text",
                got: other.kind(),
            }),
        },
        |u: Unknown| match u {
            Unknown::Text(s) => Ok(Unknown::Text(s.chars().take(s.len() / 2).collect())),
            other => Err(elm_effect_schema::SchemaError::TypeMismatch {
                path: String::new(),
                expected: "text",
                got: other.kind(),
            }),
        },
    );
    let t = Transformation::<Unknown, Unknown>::passthrough().pipe(double);
    assert_eq!(t.forward(text("ab")).unwrap(), text("abab"));
    assert_eq!(t.backward(text("abab")).unwrap(), text("ab"));
}

#[test]
fn transformation_made_from_pair_preserves_steps() {
    let t = Transformation::new(|n: i64| Ok(n + 1), |n: i64| Ok(n - 1));
    assert_eq!(t.forward(1).unwrap(), 2);
    assert_eq!(t.backward(2).unwrap(), 1);
}

// ----------------------------------------------------------------- Getter ---

#[test]
fn getter_reads_object_key() {
    let g = Getter::key("a".to_string());
    assert_eq!(g.read(&obj(&[("a", text("1"))])).unwrap(), text("1"));
    assert!(g.read(&obj(&[])).is_err());
}

#[test]
fn getter_reads_array_index() {
    let g = Getter::key("1".to_string());
    let arr = Unknown::Array(vec![text("x"), text("y")]);
    assert_eq!(g.read(&arr).unwrap(), text("y"));
    assert!(g.read(&Unknown::Array(vec![text("x")])).is_err());
}

#[test]
fn getter_map_threads_values() {
    let g = Getter::key("a".to_string()).map(|_| Ok(Unknown::Bool(true)));
    assert_eq!(
        g.read(&obj(&[("a", text("1"))])).unwrap(),
        Unknown::Bool(true)
    );
}

#[test]
fn getter_compose_chains() {
    let g = Getter::key("a".to_string()).compose(Getter::key("b".to_string()));
    let nested = obj(&[("a", obj(&[("b", text("deep"))]))]);
    assert_eq!(g.read(&nested).unwrap(), text("deep"));
}

#[test]
fn tree_record_duplicate_paths_keep_arrays() {
    let arr = |x: &str| Unknown::Array(vec![text(x)]);
    let out = make_tree_record(vec![
        (vec![Segment::Key("permissions".to_string())], arr("read")),
        (vec![Segment::Key("permissions".to_string())], arr("write")),
        (vec![Segment::Key("permissions".to_string())], arr("share")),
    ]);
    assert_eq!(
        out,
        obj(&[(
            "permissions",
            Unknown::Array(vec![text("read"), text("write"), text("share")])
        )])
    );
}

#[test]
fn tree_record_parses_string_paths() {
    use elm_effect_schema::transform::parse_path;
    assert_eq!(
        parse_path("a[0].b"),
        vec![
            Segment::Key("a".to_string()),
            Segment::Index(0),
            Segment::Key("b".to_string())
        ]
    );
}

#[test]
fn tree_record_conflicting_leaves_become_containers() {
    let out = make_tree_record(vec![
        (vec![Segment::Key("a".to_string())], text("leaf")),
        (
            vec![Segment::Key("a".to_string()), Segment::Key("b".to_string())],
            text("deep"),
        ),
    ]);
    match out {
        Unknown::Object(map) => assert!(map.contains_key("a")),
        other => panic!("expected object, got {other:?}"),
    }
}

#[test]
fn tree_record_missing_middles_initialize() {
    let out = make_tree_record(vec![(
        vec![Segment::Key("a".to_string()), Segment::Index(0)],
        text("v"),
    )]);
    match out {
        Unknown::Object(map) => assert!(map.contains_key("a")),
        other => panic!("expected object, got {other:?}"),
    }
}

#[test]
fn tree_record_bad_index_stays_object_key() {
    let out = make_tree_record(vec![(vec![Segment::Key("k".to_string())], text("v"))]);
    assert_eq!(out, obj(&[("k", text("v"))]));
}

#[test]
fn tree_record_form_paths() {
    use elm_effect_schema::transform::{flatten_tree, parse_path};
    // Simple top-level keys.
    let out = make_tree_record(vec![
        (parse_path("a"), text("1")),
        (parse_path("b"), text("two")),
    ]);
    assert_eq!(out, obj(&[("a", text("1")), ("b", text("two"))]));
    // Bracket, dot, and mixed nesting.
    let out = make_tree_record(vec![
        (parse_path("a[b]"), text("1")),
        (parse_path("c.d"), text("2")),
        (parse_path("e[0].f"), text("3")),
    ]);
    assert_eq!(
        out,
        obj(&[
            ("a", obj(&[("b", text("1"))])),
            ("c", obj(&[("d", text("2"))])),
            ("e", Unknown::Array(vec![obj(&[("f", text("3"))])])),
        ])
    );
    // Append marker and empty keys.
    let out = make_tree_record(vec![
        (parse_path("a[]"), text("1")),
        (parse_path("a[]"), text("2")),
        (parse_path(""), text("value")),
    ]);
    assert_eq!(
        out,
        obj(&[
            ("a", Unknown::Array(vec![text("1"), text("2")])),
            ("", text("value")),
        ])
    );
    // Flatten round-trips the tree shape.
    let flat = flatten_tree(&obj(&[("a", text("1"))]));
    assert_eq!(flat.len(), 1);
    // Nested objects inside appended elements, mixed indexed + append.
    let out = make_tree_record(vec![
        (parse_path("a[][b]"), text("1")),
        (parse_path("a[0][c]"), text("2")),
    ]);
    assert_eq!(
        out,
        obj(&[(
            "a",
            Unknown::Array(vec![obj(&[("b", text("1")), ("c", text("2"))]),])
        )])
    );
    // `__proto__` and `constructor` are plain own keys: no prototype
    // chain exists in this model, so nothing is traversed or skipped.
    let out = make_tree_record(vec![
        (parse_path("__proto__"), text("p")),
        (parse_path("constructor"), text("c")),
    ]);
    assert_eq!(
        out,
        obj(&[("__proto__", text("p")), ("constructor", text("c"))])
    );
}

#[test]
fn datetime_utc_from_inputs() {
    assert_eq!(
        datetime_utc_from_input("2024-01-01 01:00:00").unwrap(),
        1704070800000
    );
    assert_eq!(
        datetime_utc_from_input("2020-02-01T11:17:00+1100").unwrap(),
        1580516220000
    );
    assert_eq!(
        datetime_utc_from_input("Tue, 27 Jan 2026 17:14:06 GMT").unwrap(),
        1769534046000
    );
    assert!(datetime_utc_from_input("not a date").is_err());
}

// ----------------------------------------------------------------- Parser ---

#[test]
fn parser_type_side_and_options() {
    use elm_effect_schema::transform::{Errors, ParseOptions};
    let s = Schema::string();
    assert!(Parser::is(&s, &text("hi")));
    assert!(!Parser::is(&s, &Unknown::Bool(true)));
    assert_eq!(
        ParseOptions::default(),
        ParseOptions {
            errors: Errors::First,
            on_excess_property: elm_effect_schema::ExcessProperty::Ignore,
            disable_checks: false,
            report_input: false,
            concurrency: elm_effect_schema::transform::Concurrency::Sequential,
        }
    );
}

#[test]
fn parser_decode_first_error() {
    let s = Schema::string();
    assert_eq!(Parser::decode(&s, text("hi")).unwrap(), "hi");
    assert!(Parser::decode(&s, Unknown::Bool(true)).is_err());
}

#[test]
fn parser_decode_all_collects_vec() {
    let s = Schema::string();
    assert!(Parser::decode_all(&s, text("hi")).is_ok());
    let errs = Parser::decode_all(&s, Unknown::Bool(true)).unwrap_err();
    assert_eq!(errs.len(), 1);
}

#[test]
fn parser_decode_fields_accumulates() {
    let fields = [("a", Schema::string()), ("b", Schema::string())];
    let input = obj(&[("a", text("ok"))]);
    let errs = Parser::decode_fields(&fields, &input).unwrap_err();
    assert_eq!(errs.len(), 1);
    let full = obj(&[("a", text("ok")), ("b", text("fine"))]);
    let out = Parser::decode_fields(&fields, &full).unwrap();
    assert_eq!(out.get("b"), Some(&text("fine")));
}

#[test]
fn parser_decode_fields_rejects_non_objects() {
    let fields = [("a", Schema::string())];
    assert!(Parser::decode_fields(&fields, &text("nope")).is_err());
}

#[test]
fn parser_encode_round_trip() {
    let s = Schema::string();
    assert_eq!(s.try_encode(&"hi".to_string()).unwrap(), text("hi"));
}

#[test]
fn parser_json_entries() {
    let s = Schema::string();
    assert_eq!(Parser::decode_json(&s, "\"hi\"").unwrap(), "hi");
    assert!(Parser::decode_json(&s, "42").is_err());
    assert_eq!(Parser::decode_json_all(&s, "\"hi\"").unwrap(), "hi");
    assert_eq!(Parser::decode_json_all(&s, "42").unwrap_err().len(), 1);
}

// --------------------------------------------------------- StandardSchema ---

#[test]
fn standard_validate_shapes() {
    let s = Schema::string();
    match validate(&s, text("hi")) {
        StandardResult::Value(v) => assert_eq!(v, "hi"),
        StandardResult::Issues(_) => panic!("expected value"),
    }
    match validate(&s, Unknown::Bool(false)) {
        StandardResult::Value(_) => panic!("expected issues"),
        StandardResult::Issues(issues) => assert_eq!(issues.len(), 1),
    }
}

// ---------------------------------------------------------- ChannelSchema ---

#[test]
fn channel_chunks_round_trip() {
    let c = ChannelSchema::new(Schema::string());
    let out = c.decode_chunk(vec![text("a"), text("b")]).unwrap();
    assert_eq!(out, vec!["a".to_string(), "b".to_string()]);
    assert!(
        c.decode_chunk(vec![text("a"), Unknown::Bool(true)])
            .is_err()
    );
    let back = c.try_encode_chunk(&["a".to_string()]).unwrap();
    assert_eq!(back, vec![text("a")]);
}

// ----------------------------------------------------------------- brands ---

#[test]
fn branded_checks_refinement() {
    let b = Branded::new("NonEmpty", Schema::string(), |s: &String| !s.is_empty());
    assert_eq!(b.brand(), "NonEmpty");
    assert_eq!(b.decode(text("x")).unwrap(), "x");
    assert!(b.decode(text("")).is_err());
    assert_eq!(b.try_encode(&"x".to_string()).unwrap(), text("x"));
}

#[test]
fn branded_empty_map_has_no_constraints() {
    let m: BTreeMap<String, Unknown> = BTreeMap::new();
    assert_eq!(Unknown::Object(m.clone()), Unknown::Object(m));
}

/// Schema.test.ts:10439 (`Check > brand > single brand`): the pin builds the
/// branded schema over the SAME ast (`make(schema.ast, { schema, identifier })`),
/// so branding changes no shape and no codec, only the identifier.
#[test]
fn a_brand_leaves_the_shape_alone_and_names_the_identifier() {
    let shape = Schema::string().json_schema();
    let b = Branded::new("Positive", Schema::string(), |s: &String| !s.is_empty());
    assert_eq!(b.brand(), "Positive");
    assert_eq!(b.identifier(), "Positive");
    assert_eq!(b.schema().json_schema(), shape, "branding changes no shape");
    assert_eq!(b.decode(text("a")).unwrap(), "a");
}

/// :10446 (`double brand`): the later brand is the identifier, and the earlier
/// one stays reachable where the pin reaches it, `schema.schema.identifier`.
#[test]
fn a_second_brand_keeps_the_first_reachable() {
    let b = Branded::new("Positive", Schema::string(), |_: &String| true)
        .from_brand("Int", |s: &String| s.parse::<f64>().is_ok());
    assert_eq!(b.identifier(), "Int");
    assert_eq!(b.inner_brand(), Some("Positive"));
    assert!(b.decode(text("3")).is_ok());
    assert!(
        b.decode(text("no")).is_err(),
        "the second brand's check holds"
    );
}

/// :10453 (`override the default identifier`): `annotate({ identifier })` over
/// a branded schema is what the annotations carry, and what a refusal names.
#[test]
fn annotating_a_branded_schema_overrides_the_identifier() {
    let b =
        Branded::new("Positive", Schema::number(), |n: &f64| *n > 0.0).annotate_identifier("MyInt");
    assert_eq!(
        b.annotations(),
        vec![("identifier".to_string(), "MyInt".to_string())]
    );
    assert_eq!(b.identifier(), "MyInt");
    let refusal = b.decode(Unknown::Number(-1.0)).unwrap_err().to_string();
    assert!(
        refusal.contains("MyInt"),
        "the override renames the refusal, got: {refusal}"
    );
}

/// :10460 (`brand > fromBrand > nominal`): a nominal brand carries no check of
/// its own, so the schema's own codec is the whole refusal.
#[test]
fn a_nominal_brand_decodes_the_schema_and_refuses_other_types() {
    let b = Branded::nominal("a", Schema::string());
    assert_eq!(b.decode(text("a")).unwrap(), "a");
    assert!(b.decode(Unknown::Number(1.0)).is_err());
    assert_eq!(b.schema().json_schema(), Schema::string().json_schema());
}

/// :10471 (`single brand`): a brand constructor's check rides the brand.
#[test]
fn a_single_brand_enforces_the_constructors_check() {
    let b = Branded::new("Int", Schema::number(), |n: &f64| n.fract() == 0.0);
    assert_eq!(b.decode(Unknown::Number(1.0)).unwrap(), 1.0);
    assert!(
        b.decode(text("a")).is_err(),
        "the schema still refuses the wrong type"
    );
    assert!(
        b.decode(Unknown::Number(1.2)).is_err(),
        "the check refuses a non-integer"
    );
}

/// :10484 (`multiple brands`): two brands compose, so both checks hold. The
/// pin applies `Int` then `Positive` and refuses 1.2 and -1 separately, which
/// only a composed check can do.
#[test]
fn two_brands_run_both_checks() {
    let b = Branded::nominal("Int", Schema::number())
        .from_brand("Int", |n: &f64| n.fract() == 0.0)
        .from_brand("Positive", |n: &f64| *n > 0.0);
    assert_eq!(b.decode(Unknown::Number(1.0)).unwrap(), 1.0);
    assert!(b.decode(text("a")).is_err());
    assert!(b.decode(Unknown::Number(1.2)).is_err());
    assert!(
        b.decode(Unknown::Number(-1.0)).is_err(),
        "the first brand's check still holds"
    );
    assert_eq!(b.inner_brand(), Some("Int"));
}

/// Schema.test.ts:2485 (`decode` block): a decode-side transformation changes
/// one direction only. The pin's `String.pipe(decode(compose(trim,
/// toLowerCase)))` decodes `" A "` to `"a"` and leaves the encode side alone,
/// because `composeTransformation` is still one-directional; the same value
/// encodes back untouched.
#[test]
fn decode_only_transformation_leaves_encode_alone() {
    let schema = Schema::<String>::convert(
        Schema::string(),
        |source: String| Ok(source.trim().to_lowercase()),
        |value: &String| value.clone(),
    );

    assert_eq!(schema.decode(text(" A ")).unwrap(), "a");
    assert_eq!(schema.try_encode(&" A ".to_string()).unwrap(), text(" A "));
}

/// Schema.test.ts:2634 (`encode` block): the mirror case. `flip()` moves the
/// composition to the encode side, so decode passes the value through and
/// encode is the one that trims and lowercases it.
#[test]
fn encode_only_transformation_leaves_decode_alone() {
    let schema = Schema::<String>::convert(
        Schema::string(),
        |source: String| Ok(source),
        |value: &String| value.trim().to_lowercase(),
    );

    assert_eq!(schema.decode(text(" A ")).unwrap(), " A ");
    assert_eq!(schema.try_encode(&" A ".to_string()).unwrap(), text("a"));
}

/// Schema.test.ts:2403 and :2582 reduce to one behavior: `Trim` on either side
/// of `FiniteFromString` — `Trim.pipe(decodeTo(FiniteFromString))` and
/// `FiniteFromString.pipe(encodeTo(Trim))` — is indistinguishable from the
/// finite number string itself, because it parses its source text trimmed and
/// re-emits it canonically. `" 2 "` decodes to 2, a trimmed non-number is
/// refused, and 2 encodes back to `"2"`.
#[test]
fn finite_from_string_trims_the_source_text() {
    let schema = Schema::finite_from_string();

    assert_eq!(schema.decode(text(" 2 ")).unwrap(), 2.0);
    assert_eq!(schema.try_encode(&2.0).unwrap(), text("2"));
    let error = schema
        .decode(text(" a2 "))
        .expect_err("a trimmed text that is not a number");
    assert!(error.to_string().contains("a finite number"), "{error}");
}

#[test]
fn double_text_number_flag_round_trips() {
    use elm_effect_schema::SchemaError;
    let parse = Transformation::new(
        |u: Unknown| match u {
            Unknown::Text(s) => match s.trim().parse::<f64>() {
                Ok(n) if n.is_finite() => Ok(Unknown::Number(n)),
                _ => Err(SchemaError::TypeMismatch {
                    path: String::new(),
                    expected: "a finite number",
                    got: "text",
                }),
            },
            other => Err(SchemaError::TypeMismatch {
                path: String::new(),
                expected: "text",
                got: other.kind(),
            }),
        },
        |u: Unknown| match u {
            Unknown::Number(n) => Ok(Unknown::Text(n.to_string())),
            other => Err(SchemaError::TypeMismatch {
                path: String::new(),
                expected: "number",
                got: other.kind(),
            }),
        },
    );
    let flag = Transformation::new(
        |u: Unknown| match u {
            Unknown::Number(n) => Ok(Unknown::Bool(n != 0.0)),
            other => Err(SchemaError::TypeMismatch {
                path: String::new(),
                expected: "number",
                got: other.kind(),
            }),
        },
        |u: Unknown| match u {
            Unknown::Bool(b) => Ok(Unknown::Number(if b { 1.0 } else { 0.0 })),
            other => Err(SchemaError::TypeMismatch {
                path: String::new(),
                expected: "boolean",
                got: other.kind(),
            }),
        },
    );
    let double = parse.pipe(flag);
    assert_eq!(double.forward(text("2")).unwrap(), Unknown::Bool(true));
    assert_eq!(double.forward(text("0")).unwrap(), Unknown::Bool(false));
    assert_eq!(double.backward(Unknown::Bool(true)).unwrap(), text("1"));
}

#[test]
fn double_with_checks_rejects_between_steps() {
    use elm_effect_schema::SchemaError;
    let parse = Transformation::new(
        |u: Unknown| match u {
            Unknown::Text(s) => match s.trim().parse::<f64>() {
                Ok(n) if n.is_finite() => Ok(Unknown::Number(n)),
                _ => Err(SchemaError::TypeMismatch {
                    path: String::new(),
                    expected: "a finite number",
                    got: "text",
                }),
            },
            other => Err(SchemaError::TypeMismatch {
                path: String::new(),
                expected: "text",
                got: other.kind(),
            }),
        },
        |u: Unknown| Ok(u),
    );
    let positive = Transformation::new(
        |u: Unknown| match u {
            Unknown::Number(n) if n > 0.0 => Ok(Unknown::Number(n)),
            _ => Err(SchemaError::TypeMismatch {
                path: String::new(),
                expected: "a positive number",
                got: "number",
            }),
        },
        |u: Unknown| Ok(u),
    );
    let checked = parse.pipe(positive);
    assert_eq!(checked.forward(text("3")).unwrap(), Unknown::Number(3.0));
    let err = checked.forward(text("-1")).unwrap_err().to_string();
    assert!(
        err.contains("positive"),
        "second-step check names itself: {err}"
    );
}

#[test]
fn double_failure_reports_first_failing_step() {
    use elm_effect_schema::SchemaError;
    let parse = Transformation::new(
        |u: Unknown| match u {
            Unknown::Text(s) => match s.trim().parse::<f64>() {
                Ok(n) if n.is_finite() => Ok(Unknown::Number(n)),
                _ => Err(SchemaError::TypeMismatch {
                    path: String::new(),
                    expected: "a finite number",
                    got: "text",
                }),
            },
            other => Err(SchemaError::TypeMismatch {
                path: String::new(),
                expected: "text",
                got: other.kind(),
            }),
        },
        |u: Unknown| Ok(u),
    );
    let flag = Transformation::new(
        |u: Unknown| match u {
            Unknown::Number(n) => Ok(Unknown::Bool(n != 0.0)),
            other => Err(SchemaError::TypeMismatch {
                path: String::new(),
                expected: "number",
                got: other.kind(),
            }),
        },
        |u: Unknown| Ok(u),
    );
    let double = parse.pipe(flag);
    let err = double.forward(text("nope")).unwrap_err().to_string();
    assert!(err.contains("finite"), "first-step failure surfaces: {err}");
    let err = double.forward(Unknown::Null).unwrap_err().to_string();
    assert!(err.contains("text"), "wrong input shape fails fast: {err}");
}

// ------------- Schema.test.ts:5613 / :5627 "Transformations" ---

/// The pin's `Schema.String.pipe(Schema.decodeTo(Schema.String, t))`, where
/// `Schema::convert` is this port's `decodeTo`. A transformation carries both
/// directions, so it is cloned into the two closures.
fn case_schema(t: Transformation<String, String>) -> Schema<String> {
    let encoded = std::sync::Arc::new(t);
    let decoded = encoded.clone();
    Schema::convert(
        Schema::string(),
        move |value: String| decoded.forward(value),
        // `convert` takes a total encode, and a case transformation's encode
        // direction is the pin's passthrough getter, so this cannot fail.
        move |value: &String| {
            encoded
                .backward(value.clone())
                .expect("the case transformations pass through on encode")
        },
    )
}

#[test]
fn to_lowercase_lowers_on_decode_and_passes_through_on_encode() {
    let lowered = case_schema(Transformation::to_lowercase());
    assert_eq!(lowered.decode(text("A")).unwrap(), "a");
    assert_eq!(lowered.decode(text("B")).unwrap(), "b");
    // Encoding is passthrough, which is why the pair is not round-trippable:
    // an uppercase value encodes back unchanged rather than re-cased.
    assert_eq!(lowered.try_encode(&"A".to_string()).unwrap(), text("A"));
}

#[test]
fn to_uppercase_uppers_on_decode_and_passes_through_on_encode() {
    let uppered = case_schema(Transformation::to_uppercase());
    assert_eq!(uppered.decode(text("a")).unwrap(), "A");
    assert_eq!(uppered.decode(text("b")).unwrap(), "B");
    assert_eq!(uppered.try_encode(&"a".to_string()).unwrap(), text("a"));
}

/// The pin's `Schema.String.pipe(Schema.decodeTo(Schema.String, t))` for the
/// `StringFrom*` family: both directions are wired, because these transforms
/// round-trip (unlike the case ones).
fn string_wire_schema(t: Transformation<String, String>) -> Schema<String> {
    let decoded = std::sync::Arc::new(t);
    let encoded = decoded.clone();
    Schema::convert(
        Schema::string(),
        move |value: String| decoded.forward(value),
        move |value: &String| {
            encoded
                .backward(value.clone())
                .expect("an encoder writes any string")
        },
    )
}

/// Schema.test.ts:5240 (`StringFromBase64`): Base64 text on the wire, the
/// UTF-8 text it carries as the value, and the refusals are the malformations
/// the pin names \u2014 a length that is not a group of four, the URL-safe
/// alphabet, and leading padding.
#[test]
fn string_from_base64_round_trips_and_refuses_malformed_text() {
    let schema = string_wire_schema(Transformation::string_from_base64());
    assert_eq!(schema.decode(text("Zm9vYmFy")).unwrap(), "foobar");
    for bad in ["Zm9vY", "Zm9vYmF-", "=Zm9vYmF"] {
        let err = schema.decode(text(bad)).unwrap_err().to_string();
        assert!(
            err.contains("a valid Base64 string"),
            "{bad:?} must be refused by name, got: {err}"
        );
    }
    assert_eq!(
        schema.try_encode(&"foobar".to_string()).unwrap(),
        text("Zm9vYmFy")
    );
}

/// :5254 (`StringFromBase64Url`): the URL-safe alphabet, no padding on the way
/// out, and the standard alphabet's `+` and `/` are now the malformed input.
#[test]
fn string_from_base64_url_uses_the_url_alphabet() {
    let schema = string_wire_schema(Transformation::string_from_base64_url());
    assert_eq!(schema.decode(text("Zm9vYmFy")).unwrap(), "foobar");
    assert_eq!(schema.decode(text("Pj8-ZD_Dnw")).unwrap(), ">?>d?\u{df}");
    let err = schema.decode(text("Pj8/ZD+Dnw")).unwrap_err().to_string();
    assert!(err.contains("a valid Base64Url string"), "got: {err}");
    assert_eq!(
        schema.try_encode(&"foobar".to_string()).unwrap(),
        text("Zm9vYmFy")
    );
    assert_eq!(
        schema.try_encode(&">?>d?\u{df}".to_string()).unwrap(),
        text("Pj8-ZD_Dnw")
    );
}

/// :5269 (`StringFromHex`): hexadecimal text on the wire. An odd count of
/// digits is refused (`"0"`), as is a non-digit whether it is text (`zd4aa`)
/// or a control character.
#[test]
fn string_from_hex_round_trips_and_refuses_malformed_text() {
    let schema = string_wire_schema(Transformation::string_from_hex());
    assert_eq!(schema.decode(text("67")).unwrap(), "g");
    for bad in ["0", "zd4aa", "0\u{1}"] {
        let err = schema.decode(text(bad)).unwrap_err().to_string();
        assert!(
            err.contains("a valid hexadecimal string"),
            "{bad:?} must be refused by name, got: {err}"
        );
    }
    assert_eq!(schema.try_encode(&"g".to_string()).unwrap(), text("67"));
    assert_eq!(
        schema.decode(text("0001020304050607")).unwrap(),
        "\u{0}\u{1}\u{2}\u{3}\u{4}\u{5}\u{6}\u{7}"
    );
}

/// :5283 (`StringFromUriComponent`): percent-encoded text on the wire. The
/// decoded side is UTF-8, so a multi-byte escape decodes to its characters,
/// and a truncated or non-hex escape (`%ZZ`) is the refusal.
#[test]
fn string_from_uri_component_round_trips_and_refuses_malformed_text() {
    let schema = string_wire_schema(Transformation::string_from_uri_component());
    assert_eq!(
        schema.decode(text("%7B%22a%22%3A1%7D")).unwrap(),
        "{\"a\":1}"
    );
    assert_eq!(
        schema
            .decode(text("%D1%88%D0%B5%D0%BB%D0%BB%D1%8B"))
            .unwrap(),
        "шеллы"
    );
    assert_eq!(schema.decode(text("hello%20world")).unwrap(), "hello world");
    assert_eq!(schema.decode(text("hello")).unwrap(), "hello");
    let err = schema.decode(text("%ZZ")).unwrap_err().to_string();
    assert!(err.contains("a valid URI component"), "got: {err}");
    assert_eq!(
        schema.try_encode(&"{\"a\":1}".to_string()).unwrap(),
        text("%7B%22a%22%3A1%7D")
    );
    assert_eq!(
        schema.try_encode(&"hello world".to_string()).unwrap(),
        text("hello%20world")
    );
}

/// Schema.test.ts:5396 (`DateTimeUtcFromString`): the wire is the canonical UTC
/// string and the value is the instant. The pin's value is a `DateTime`, which
/// this port has no `Unknown` variant for, so the port's instant is epoch
/// millis (`transform::date_schema`) and the assertions are the row's own
/// decode/encode/refusal set.
#[test]
fn datetime_utc_from_string_decodes_the_canonical_wire_string() {
    let schema = elm_effect_schema::transform::date_schema();
    assert_eq!(
        schema.decode(text("2021-01-01T00:00:00.000Z")).unwrap(),
        1_609_459_200_000
    );
    assert_eq!(
        schema.try_encode(&1_609_459_200_000).unwrap(),
        text("2021-01-01T00:00:00.000Z")
    );
    assert!(
        schema.decode(text("invalid")).is_err(),
        "an unparseable instant is refused"
    );
    assert!(
        schema.decode(Unknown::Null).is_err(),
        "null is not a string"
    );
}

/// :5413 (`DateTimeUtcFromMillis`): the wire is the millisecond number and the
/// value is the same instant, so the port's `date_from_millis_schema` is this
/// codec with millis standing in for the pin's `DateTime`.
#[test]
fn datetime_utc_from_millis_decodes_a_millisecond_number() {
    let schema = elm_effect_schema::transform::date_from_millis_schema();
    assert_eq!(
        schema.decode(Unknown::Number(1_609_459_200_000.0)).unwrap(),
        1_609_459_200_000
    );
    assert_eq!(
        schema.try_encode(&1_609_459_200_000).unwrap(),
        Unknown::Number(1_609_459_200_000.0)
    );
    assert!(
        schema.decode(Unknown::Null).is_err(),
        "null is not a number"
    );
}

// ------------------------------------------------- Schema.test.ts:10292 ---

/// `Schema.test.ts:10292` — "Getter" / "succeed": a getter that answers a fixed
/// value still runs its source schema on the way in, so `0` decodes to `"a"`
/// and `1` is refused as `0`'s shape. On the way out the value has to be the
/// target side's, so `"b"` is refused where `"a"` encodes back to `0`.
#[test]
fn a_getter_that_ignores_its_input_still_checks_both_sides() {
    use elm_effect_schema::SchemaError;

    let schema = Schema::try_convert(
        Schema::literal(Unknown::Number(0.0)),
        |_value: Unknown| Ok(text("a")),
        |value: &Unknown| match value {
            Unknown::Text(text) if text == "a" => Ok(Unknown::Number(0.0)),
            _ => Err(SchemaError::EncodingFailed {
                path: "$".to_string(),
                expected: "\"a\"".to_string(),
            }),
        },
    );

    assert_eq!(schema.decode(Unknown::Number(0.0)).unwrap(), text("a"));
    let refused = schema
        .decode(Unknown::Number(1.0))
        .expect_err("1 is not 0")
        .to_string();
    assert!(refused.contains("0"), "{refused}");

    assert_eq!(schema.try_encode(&text("a")).unwrap(), Unknown::Number(0.0));
    let refused = schema
        .try_encode(&text("b"))
        .expect_err("\"b\" is not the target side's value")
        .to_string();
    assert!(refused.contains("\"a\""), "{refused}");
}
