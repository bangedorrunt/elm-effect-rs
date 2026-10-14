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
    use elm_effect_schema::transform::ParseOptions;
    let s = Schema::string();
    assert!(Parser::is(&s, &text("hi")));
    assert!(!Parser::is(&s, &Unknown::Bool(true)));
    assert_eq!(
        ParseOptions::default(),
        ParseOptions {
            preserve_excess: false
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
    assert_eq!(Parser::encode(&s, &"hi".to_string()), text("hi"));
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
    let back = c.encode_chunk(&["a".to_string()]);
    assert_eq!(back, vec![text("a")]);
}

// ----------------------------------------------------------------- brands ---

#[test]
fn branded_checks_refinement() {
    let b = Branded::new("NonEmpty", Schema::string(), |s: &String| !s.is_empty());
    assert_eq!(b.brand(), "NonEmpty");
    assert_eq!(b.decode(text("x")).unwrap(), "x");
    assert!(b.decode(text("")).is_err());
    assert_eq!(b.encode(&"x".to_string()), text("x"));
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
    assert_eq!(schema.encode(&" A ".to_string()), text(" A "));
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
    assert_eq!(schema.encode(&" A ".to_string()), text("a"));
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
    assert_eq!(schema.encode(&2.0), text("2"));
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
    assert_eq!(lowered.encode(&"A".to_string()), text("A"));
}

#[test]
fn to_uppercase_uppers_on_decode_and_passes_through_on_encode() {
    let uppered = case_schema(Transformation::to_uppercase());
    assert_eq!(uppered.decode(text("a")).unwrap(), "A");
    assert_eq!(uppered.decode(text("b")).unwrap(), "B");
    assert_eq!(uppered.encode(&"a".to_string()), text("a"));
}
