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
    Unknown::Object(pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect())
}

fn text(s: &str) -> Unknown {
    Unknown::Text(s.to_string())
}

// ------------------------------------------------------- Transformation ---

#[test]
fn is_transformation_tags_structures() {
    assert!(is_transformation(&Transformation::<Unknown, Unknown>::passthrough()));
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
    let t = Transformation::new(
        |n: i64| Ok(n + 1),
        |n: i64| Ok(n - 1),
    );
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
    let g = Getter::key("a".to_string())
        .map(|_| Ok(Unknown::Bool(true)));
    assert_eq!(g.read(&obj(&[("a", text("1"))])).unwrap(), Unknown::Bool(true));
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
    assert_eq!(out, obj(&[("permissions", Unknown::Array(vec![text("read"), text("write"), text("share")]))]));
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
    let out = make_tree_record(vec![(
        vec![Segment::Key("k".to_string())],
        text("v"),
    )]);
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
        obj(&[("a", Unknown::Array(vec![
            obj(&[("b", text("1")), ("c", text("2"))]),
        ]))])
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
    assert_eq!(datetime_utc_from_input("2024-01-01 01:00:00").unwrap(), 1704070800000);
    assert_eq!(datetime_utc_from_input("2020-02-01T11:17:00+1100").unwrap(), 1580516220000);
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
    assert_eq!(ParseOptions::default(), ParseOptions { preserve_excess: false });
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
    let fields = [
        ("a", Schema::string()),
        ("b", Schema::string()),
    ];
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
    assert!(c.decode_chunk(vec![text("a"), Unknown::Bool(true)]).is_err());
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
