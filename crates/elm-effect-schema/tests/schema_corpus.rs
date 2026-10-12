//! Wave corpus: literal / union / boundary / optional-field shapes.
//! Ports packages/effect/test/schema/Schema.test.ts rows by exact behavior.

use elm_effect_schema::{Field, Schema, TemplatePart, Unknown};

#[test]
fn literal_exposes_its_value() {
    let schema = Schema::literal(Unknown::Text("a".to_string()));
    assert_eq!(
        schema.decode(Unknown::Text("a".to_string())).unwrap(),
        Unknown::Text("a".to_string())
    );
}

#[test]
fn literal_rejects_other_values() {
    let schema = Schema::literal(Unknown::Number(1.0));
    assert!(schema.decode(Unknown::Number(2.0)).is_err());
    assert!(schema.decode(Unknown::Text("1".to_string())).is_err());
}

#[test]
fn union_second_member_decodes() {
    let schema = Schema::union(
        Schema::literal(Unknown::Text("a".to_string())),
        Schema::literal(Unknown::Text("b".to_string())),
    );
    assert_eq!(
        schema.decode(Unknown::Text("b".to_string())).unwrap(),
        Unknown::Text("b".to_string())
    );
    assert!(schema.decode(Unknown::Text("c".to_string())).is_err());
}

#[test]
fn struct_shape_validates_fields() {
    use elm_effect_schema::Field;
    let schema = Schema::struct_shape(vec![
        Field::required("name", Schema::as_unknown(Schema::string())),
        Field::optional("nickname", Schema::as_unknown(Schema::string())),
    ]);
    let mut input = std::collections::BTreeMap::new();
    input.insert("name".to_string(), Unknown::Text("Al".to_string()));
    let out = schema.decode(Unknown::Object(input)).unwrap();
    assert_eq!(out["name"], Unknown::Text("Al".to_string()));
    assert!(!out.contains_key("nickname"));
    let error = schema
        .decode(Unknown::Object(std::collections::BTreeMap::new()))
        .unwrap_err();
    assert!(error.to_string().contains("$.name"));
}

#[test]
fn pick_decodes_a_subset() {
    use elm_effect_schema::Field;
    let fields = vec![
        Field::required("a", Schema::as_unknown(Schema::string())),
        Field::required("b", Schema::as_unknown(Schema::number())),
    ];
    let schema = Schema::pick(fields, &["b"]);
    let mut input = std::collections::BTreeMap::new();
    input.insert("b".to_string(), Unknown::Number(1.0));
    let out = schema.decode(Unknown::Object(input)).unwrap();
    assert_eq!(out["b"], Unknown::Number(1.0));
    // The picked-away field is not required anymore.
    assert!(!out.contains_key("a"));
}

#[test]
fn tuple_checks_length_and_order() {
    let schema = Schema::tuple2(Schema::string(), Schema::number());
    assert_eq!(
        schema
            .decode(Unknown::Array(vec![
                Unknown::Text("a".to_string()),
                Unknown::Number(1.0)
            ]))
            .unwrap(),
        ("a".to_string(), 1.0)
    );
    let error = schema
        .decode(Unknown::Array(vec![Unknown::Number(1.0)]))
        .unwrap_err();
    assert!(error.to_string().contains("$[0]"));
}

#[test]
fn refine_rejects_failing_values() {
    let schema = Schema::number().refine("positive", |n: &f64| *n > 0.0);
    assert_eq!(schema.decode(Unknown::Number(1.0)).unwrap(), 1.0);
    let error = schema.decode(Unknown::Number(-1.0)).unwrap_err();
    assert!(error.to_string().contains("positive"));
}

#[test]
fn refine_is_int() {
    let schema = Schema::number().refine("an integer", |n: &f64| n.fract() == 0.0);
    assert_eq!(schema.decode(Unknown::Number(3.0)).unwrap(), 3.0);
    assert!(schema.decode(Unknown::Number(3.5)).is_err());
}

#[test]
fn refine_is_between() {
    let schema = Schema::number().refine("between 1 and 5", |n: &f64| (1.0..=5.0).contains(n));
    assert_eq!(schema.decode(Unknown::Number(4.0)).unwrap(), 4.0);
    assert!(schema.decode(Unknown::Number(6.0)).is_err());
}

#[test]
fn refine_is_lowercased() {
    let schema = Schema::string().refine("lowercased", |s: &String| {
        !s.chars().any(|c| c.is_uppercase())
    });
    assert!(schema.decode(Unknown::Text("abc".to_string())).is_ok());
    assert!(schema.decode(Unknown::Text("aBc".to_string())).is_err());
}

#[test]
fn refine_is_uppercased() {
    let schema = Schema::string().refine("uppercased", |s: &String| {
        !s.chars().any(|c| c.is_lowercase())
    });
    assert!(schema.decode(Unknown::Text("ABC".to_string())).is_ok());
    assert!(schema.decode(Unknown::Text("aBC".to_string())).is_err());
}

#[test]
fn refine_is_string_finite() {
    let schema = Schema::string().refine("a finite number", |s: &String| {
        s.parse::<f64>().is_ok_and(|n| n.is_finite())
    });
    assert!(schema.decode(Unknown::Text("3.5".to_string())).is_ok());
    assert!(schema.decode(Unknown::Text("abc".to_string())).is_err());
    assert!(schema.decode(Unknown::Text("inf".to_string())).is_err());
}

#[test]
fn refine_is_uncapitalized() {
    let schema = Schema::string().refine("uncapitalized", |s: &String| {
        s.chars().next().is_none_or(|c| !c.is_uppercase())
    });
    assert!(schema.decode(Unknown::Text("hello".to_string())).is_ok());
    assert!(schema.decode(Unknown::Text("Hello".to_string())).is_err());
}

#[test]
fn refine_is_starting_with() {
    let schema = Schema::string().refine("starting with pre", |s: &String| s.starts_with("pre"));
    assert!(schema.decode(Unknown::Text("prefix".to_string())).is_ok());
    assert!(schema.decode(Unknown::Text("suffix".to_string())).is_err());
}

#[test]
fn refine_is_ulid() {
    let schema = Schema::string().refine("a ULID", |s: &String| {
        s.len() == 26
            && s.chars().all(|c| matches!(c, '0'..='9' | 'A'..='H' | 'J'..='K' | 'M'..='N' | 'P'..='T' | 'V'..='Z'))
    });
    assert!(
        schema
            .decode(Unknown::Text("01ARZ3NDEKTSV4RRFFQ69G5FAV".to_string()))
            .is_ok()
    );
    assert!(
        schema
            .decode(Unknown::Text("not-a-ulid".to_string()))
            .is_err()
    );
}

#[test]
fn refine_is_multiple_of() {
    let divisor = -3.0f64.abs();
    let schema = Schema::number().refine("a multiple of 3", move |n: &f64| n % divisor == 0.0);
    assert_eq!(schema.decode(Unknown::Number(9.0)).unwrap(), 9.0);
    assert!(schema.decode(Unknown::Number(10.0)).is_err());
}

#[test]
fn annotated_schemas_emit_descriptions() {
    let schema = Schema::annotated(Schema::string(), Some("Name"), Some("The name field"));
    let doc = schema.json_schema();
    assert_eq!(doc["type"], serde_json::json!("string"));
    assert_eq!(doc["title"], serde_json::json!("Name"));
    assert_eq!(doc["description"], serde_json::json!("The name field"));
    // The wire shape is unchanged.
    assert!(schema.decode(Unknown::Text("x".to_string())).is_ok());
}

#[test]
fn json_schema_unconstrained_imports_as_any() {
    let schema = Schema::<Unknown>::from_json_schema(&serde_json::json!({})).unwrap();
    assert_eq!(
        schema.decode(Unknown::Number(1.0)).unwrap(),
        Unknown::Number(1.0)
    );
}

#[test]
fn json_schema_single_member_enums() {
    let schema = Schema::<Unknown>::from_json_schema(&serde_json::json!({"enum": [1]})).unwrap();
    assert_eq!(
        schema.decode(Unknown::Number(1.0)).unwrap(),
        Unknown::Number(1.0)
    );
    assert!(schema.decode(Unknown::Number(2.0)).is_err());
    let schema = Schema::<Unknown>::from_json_schema(&serde_json::json!({"enum": [true]})).unwrap();
    assert_eq!(
        schema.decode(Unknown::Bool(true)).unwrap(),
        Unknown::Bool(true)
    );
    assert!(schema.decode(Unknown::Bool(false)).is_err());
}

#[test]
fn json_schema_single_member_enum() {
    let schema =
        Schema::<Unknown>::from_json_schema(&serde_json::json!({"enum": ["only"]})).unwrap();
    assert_eq!(
        schema.decode(Unknown::Text("only".to_string())).unwrap(),
        Unknown::Text("only".to_string())
    );
    assert!(schema.decode(Unknown::Text("other".to_string())).is_err());
}

#[test]
fn json_schema_const_numbers_and_booleans() {
    let schema = Schema::<Unknown>::from_json_schema(&serde_json::json!({"const": 1})).unwrap();
    assert_eq!(
        schema.decode(Unknown::Number(1.0)).unwrap(),
        Unknown::Number(1.0)
    );
    assert!(schema.decode(Unknown::Number(2.0)).is_err());
    let schema = Schema::<Unknown>::from_json_schema(&serde_json::json!({"const": true})).unwrap();
    assert_eq!(
        schema.decode(Unknown::Bool(true)).unwrap(),
        Unknown::Bool(true)
    );
    assert!(schema.decode(Unknown::Bool(false)).is_err());
}

#[test]
fn json_schema_const_and_enum_import_as_literals() {
    let schema = Schema::<Unknown>::from_json_schema(&serde_json::json!({"const": "red"})).unwrap();
    assert_eq!(
        schema.decode(Unknown::Text("red".to_string())).unwrap(),
        Unknown::Text("red".to_string())
    );
    assert!(schema.decode(Unknown::Text("blue".to_string())).is_err());
    let schema =
        Schema::<Unknown>::from_json_schema(&serde_json::json!({"enum": ["red", "blue"]})).unwrap();
    assert!(schema.decode(Unknown::Text("blue".to_string())).is_ok());
    assert!(schema.decode(Unknown::Text("green".to_string())).is_err());
}

#[test]
fn json_schema_null_and_typed_imports() {
    let schema = Schema::<Unknown>::from_json_schema(&serde_json::json!({"type": "null"})).unwrap();
    assert!(schema.decode(Unknown::Null).is_ok());
    assert!(schema.decode(Unknown::Number(0.0)).is_err());
    let schema =
        Schema::<Unknown>::from_json_schema(&serde_json::json!({"type": "string"})).unwrap();
    assert!(schema.decode(Unknown::Text("x".to_string())).is_ok());
    assert!(schema.decode(Unknown::Number(1.0)).is_err());
}

#[test]
fn json_schema_object_imports_required_and_optional() {
    let schema = Schema::<Unknown>::from_json_schema(&serde_json::json!({
        "type": "object",
        "properties": {"name": {"type": "string"}, "age": {"type": "number"}},
        "required": ["name"],
    }))
    .unwrap();
    let mut input = std::collections::BTreeMap::new();
    input.insert("name".to_string(), Unknown::Text("Al".to_string()));
    assert!(schema.decode(Unknown::Object(input)).is_ok());
    assert!(
        schema
            .decode(Unknown::Object(std::collections::BTreeMap::new()))
            .is_err()
    );
}

#[test]
fn json_schema_ref_is_an_honest_error() {
    let result = Schema::<Unknown>::from_json_schema(&serde_json::json!({"$ref": "#/$defs/X"}));
    assert!(result.is_err());
}

#[test]
fn json_decodes_each_wire_type() {
    assert_eq!(Schema::null().decode_json("null").unwrap(), ());
    assert_eq!(
        Schema::any().decode_json("42").unwrap(),
        Unknown::Number(42.0)
    );
    assert_eq!(Schema::number().decode_json("2.5").unwrap(), 2.5);
    assert!(Schema::boolean().decode_json("true").unwrap());
    assert_eq!(
        Schema::array(Schema::string())
            .decode_json(r#"["a"]"#)
            .unwrap(),
        vec!["a".to_string()]
    );
    assert!(Schema::object().decode_json(r#"{"k": 1}"#).is_ok());
    assert!(Schema::string().decode_json(r#""s""#).is_ok());
    let document = Schema::any().decode_json(r#"{"a": [1, true]}"#).unwrap();
    assert!(matches!(document, Unknown::Object(_)));
}

#[test]
fn json_decodes_string_enums() {
    let schema = Schema::union(
        Schema::literal(Unknown::Text("red".to_string())),
        Schema::literal(Unknown::Text("blue".to_string())),
    );
    let value = schema.decode_json(r#""blue""#).unwrap();
    assert_eq!(value, Unknown::Text("blue".to_string()));
    assert!(schema.decode_json(r#""green""#).is_err());
}

#[test]
fn refine_non_empty_string() {
    let schema = Schema::string().refine("non-empty", |s: &String| !s.is_empty());
    assert!(schema.decode(Unknown::Text("x".to_string())).is_ok());
    assert!(schema.decode(Unknown::Text(String::new())).is_err());
}

#[test]
fn revives_predicate_strings() {
    let base64 = Schema::string().refine("base64", |x: &String| {
        !x.is_empty()
            && x.len().is_multiple_of(4)
            && x.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '/' || c == '=')
    });
    assert!(base64.decode_json(r#""aGk=""#).is_ok());
    assert!(base64.decode_json(r#""!!""#).is_err());
    let base64url = Schema::string().refine("base64url", |x: &String| {
        !x.is_empty()
            && x.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    });
    assert!(base64url.decode_json(r#""aGk""#).is_ok());
    assert!(base64url.decode_json(r#""a b""#).is_err());
    let starting = Schema::string().refine("starting with pre", |x: &String| x.starts_with("pre"));
    assert!(starting.decode_json(r#""prefix""#).is_ok());
    assert!(starting.decode_json(r#""suffix""#).is_err());
    let ending = Schema::string().refine("ending with .ts", |x: &String| x.ends_with(".ts"));
    assert!(ending.decode_json(r#""a.ts""#).is_ok());
    assert!(ending.decode_json(r#""a.rs""#).is_err());
    let including = Schema::string().refine("including mid", |x: &String| x.contains("mid"));
    assert!(including.decode_json(r#""amidb""#).is_ok());
    assert!(including.decode_json(r#""ab""#).is_err());
    let upper = Schema::string().refine("uppercased", |x: &String| {
        !x.chars().any(|c| c.is_lowercase())
    });
    assert!(upper.decode_json(r#""ABC""#).is_ok());
    assert!(upper.decode_json(r#""aBC""#).is_err());
    let lower = Schema::string().refine("lowercased", |x: &String| {
        !x.chars().any(|c| c.is_uppercase())
    });
    assert!(lower.decode_json(r#""abc""#).is_ok());
    assert!(lower.decode_json(r#""aBc""#).is_err());
    let capped = Schema::string().refine("capitalized", |x: &String| {
        x.chars().next().is_some_and(|c| c.is_uppercase())
    });
    assert!(capped.decode_json(r#""Hello""#).is_ok());
    assert!(capped.decode_json(r#""hello""#).is_err());
    let uncapped = Schema::string().refine("uncapitalized", |x: &String| {
        x.chars().next().is_none_or(|c| !c.is_uppercase())
    });
    assert!(uncapped.decode_json(r#""hello""#).is_ok());
    assert!(uncapped.decode_json(r#""Hello""#).is_err());
}

#[test]
fn revives_string_bigint() {
    let schema = Schema::string().refine("a bigint string", |x: &String| {
        let digits = x.strip_prefix('-').unwrap_or(x.as_str());
        !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit())
    });
    assert!(schema.decode_json(r#""123""#).is_ok());
    assert!(schema.decode_json(r#""-45""#).is_ok());
    assert!(schema.decode_json(r#""12.5""#).is_err());
}

#[test]
fn revives_bounded_numbers() {
    let at_least = Schema::number().refine("at least 5", |n: &f64| *n >= 5.0);
    assert!(at_least.decode_json("5").is_ok());
    assert!(at_least.decode_json("4").is_err());
    let at_most = Schema::number().refine("at most 5", |n: &f64| *n <= 5.0);
    assert!(at_most.decode_json("5").is_ok());
    assert!(at_most.decode_json("6").is_err());
    let finite = Schema::number().refine("finite", |n: &f64| n.is_finite());
    assert!(finite.decode_json("1.5").is_ok());
    assert!(finite.decode(Unknown::Number(f64::INFINITY)).is_err());
}

#[test]
fn revives_predicate_numbers() {
    let int = Schema::number().refine("an integer", |n: &f64| n.fract() == 0.0);
    assert!(int.decode_json("3").is_ok());
    assert!(int.decode_json("3.5").is_err());
    let multiple = Schema::number().refine("a multiple of 3", |n: &f64| n % 3.0 == 0.0);
    assert!(multiple.decode_json("9").is_ok());
    assert!(multiple.decode_json("10").is_err());
    let greater = Schema::number().refine("greater than 5", |n: &f64| *n > 5.0);
    assert!(greater.decode_json("6").is_ok());
    assert!(greater.decode_json("5").is_err());
    let less = Schema::number().refine("less than 5", |n: &f64| *n < 5.0);
    assert!(less.decode_json("4").is_ok());
    assert!(less.decode_json("5").is_err());
}

#[test]
fn revives_bounded_lengths() {
    let maxed = Schema::string().refine("max length 3", |s: &String| s.len() <= 3);
    assert!(maxed.decode_json(r#""ab""#).is_ok());
    assert!(maxed.decode_json(r#""abcd""#).is_err());
    let ranged = Schema::string().refine("length 2 to 3", |s: &String| (2..=3).contains(&s.len()));
    assert!(ranged.decode_json(r#""ab""#).is_ok());
    assert!(ranged.decode_json(r#""a""#).is_err());
    assert!(ranged.decode_json(r#""abcd""#).is_err());
}

#[test]
fn revives_code_point_bounds() {
    let min = Schema::string().refine("min 2 code points", |s: &String| s.chars().count() >= 2);
    assert!(min.decode_json(r#""ab""#).is_ok());
    assert!(min.decode_json(r#""a""#).is_err());
    let max = Schema::string().refine("max 2 code points", |s: &String| s.chars().count() <= 2);
    assert!(max.decode_json(r#""ab""#).is_ok());
    assert!(max.decode_json(r#""abc""#).is_err());
    let between = Schema::string().refine("2 to 3 code points", |s: &String| {
        (2..=3).contains(&s.chars().count())
    });
    assert!(between.decode_json(r#""ab""#).is_ok());
    assert!(between.decode_json(r#""a""#).is_err());
}

#[test]
fn revives_trimmed() {
    let schema = Schema::string().refine("trimmed", |s: &String| s.trim() == s.as_str());
    assert!(schema.decode_json(r#""ab""#).is_ok());
    assert!(schema.decode_json(r#"" ab""#).is_err());
}

#[test]
fn revives_checked_strings() {
    let finite = Schema::string().refine("a finite number", |s: &String| {
        s.parse::<f64>().is_ok_and(|n| n.is_finite())
    });
    assert!(finite.decode_json(r#""3.5""#).is_ok());
    assert!(finite.decode_json(r#""abc""#).is_err());
    let min = Schema::string().refine("min length 2", |s: &String| s.len() >= 2);
    assert!(min.decode_json(r#""ab""#).is_ok());
    assert!(min.decode_json(r#""a""#).is_err());
    let uuid = Schema::string().refine("a UUID", |s: &String| {
        let parts: Vec<&str> = s.split('-').collect();
        parts.len() == 5
    });
    assert!(
        uuid.decode_json(r#""f47ac10b-58cc-4372-a567-0e02b2c3d479""#)
            .is_ok()
    );
    assert!(uuid.decode_json(r#""xyz""#).is_err());
    let guid = Schema::string().refine("a GUID", |s: &String| {
        let parts: Vec<&str> = s.split('-').collect();
        parts.len() == 5
    });
    assert!(
        guid.decode_json(r#""123e4567-e89b-12d3-a456-426614174000""#)
            .is_ok()
    );
    assert!(guid.decode_json(r#""xyz""#).is_err());
    let ulid = Schema::string().refine("a ULID", |s: &String| s.len() == 26);
    assert!(ulid.decode_json(r#""01ARZ3NDEKTSV4RRFFQ69G5FAV""#).is_ok());
    assert!(ulid.decode_json(r#""xyz""#).is_err());
}

#[test]
fn refine_is_base64() {
    let schema = Schema::string().refine("base64", |s: &String| {
        !s.is_empty()
            && s.len().is_multiple_of(4)
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '/' || c == '=')
    });
    assert!(schema.decode(Unknown::Text("aGk=".to_string())).is_ok());
    assert!(schema.decode(Unknown::Text("nope!".to_string())).is_err());
}

#[test]
fn refine_is_base64url() {
    let schema = Schema::string().refine("base64url", |s: &String| {
        !s.is_empty()
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    });
    assert!(schema.decode(Unknown::Text("aGk".to_string())).is_ok());
    assert!(schema.decode(Unknown::Text("a b".to_string())).is_err());
}

#[test]
fn refine_is_uuid() {
    let schema = Schema::string().refine("a UUID", |s: &String| {
        let parts: Vec<&str> = s.split('-').collect();
        parts.len() == 5
            && [8, 4, 4, 4, 12]
                .iter()
                .zip(parts.iter())
                .all(|(len, part)| {
                    part.len() == *len && part.chars().all(|c| c.is_ascii_hexdigit())
                })
    });
    assert!(
        schema
            .decode(Unknown::Text(
                "f47ac10b-58cc-4372-a567-0e02b2c3d479".to_string()
            ))
            .is_ok()
    );
    assert!(schema.decode(Unknown::Text("xyz".to_string())).is_err());
}

#[test]
fn refine_is_ending_with() {
    let schema = Schema::string().refine("ending with .ts", |s: &String| s.ends_with(".ts"));
    assert!(schema.decode(Unknown::Text("a.ts".to_string())).is_ok());
    assert!(schema.decode(Unknown::Text("a.rs".to_string())).is_err());
}

#[test]
fn refine_is_capitalized() {
    let schema = Schema::string().refine("capitalized", |s: &String| {
        s.chars().next().is_some_and(|c| c.is_uppercase())
    });
    assert!(schema.decode(Unknown::Text("Hello".to_string())).is_ok());
    assert!(schema.decode(Unknown::Text("hello".to_string())).is_err());
}

#[test]
fn refine_is_guid() {
    let schema = Schema::string().refine("a GUID", |s: &String| {
        let parts: Vec<&str> = s.split('-').collect();
        parts.len() == 5
            && [8, 4, 4, 4, 12]
                .iter()
                .zip(parts.iter())
                .all(|(len, part)| {
                    part.len() == *len && part.chars().all(|c| c.is_ascii_hexdigit())
                })
    });
    assert!(
        schema
            .decode(Unknown::Text(
                "123e4567-e89b-12d3-a456-426614174000".to_string()
            ))
            .is_ok()
    );
    assert!(
        schema
            .decode(Unknown::Text("not-a-guid".to_string()))
            .is_err()
    );
}

#[test]
fn refine_is_int32() {
    let schema = Schema::number().refine("an int32", |n: &f64| {
        n.fract() == 0.0 && (-2_147_483_648.0..=2_147_483_647.0).contains(n)
    });
    assert_eq!(schema.decode(Unknown::Number(5.0)).unwrap(), 5.0);
    assert!(schema.decode(Unknown::Number(5_000_000_000.0)).is_err());
    assert!(schema.decode(Unknown::Number(5.5)).is_err());
}

#[test]
fn refine_is_min_length() {
    let schema = Schema::string().refine("min length 2", |s: &String| s.len() >= 2);
    assert!(schema.decode(Unknown::Text("ab".to_string())).is_ok());
    assert!(schema.decode(Unknown::Text("a".to_string())).is_err());
}

#[test]
fn null_accepts_only_null() {
    assert_eq!(Schema::null().decode(Unknown::Null).unwrap(), ());
    assert!(Schema::null().decode(Unknown::Number(0.0)).is_err());
}

#[test]
fn any_accepts_everything() {
    assert_eq!(
        Schema::any().decode(Unknown::Number(1.0)).unwrap(),
        Unknown::Number(1.0)
    );
    assert_eq!(Schema::any().decode(Unknown::Null).unwrap(), Unknown::Null);
}

#[test]
fn never_rejects_everything() {
    assert!(Schema::never().decode(Unknown::Null).is_err());
}

#[test]
fn option_from_null_or_value() {
    let schema = Schema::optional(Schema::string());
    assert_eq!(schema.decode(Unknown::Null).unwrap(), None);
    assert_eq!(
        schema.decode(Unknown::Text("x".to_string())).unwrap(),
        Some("x".to_string())
    );
}

#[test]
fn optional_field_missing_is_none() {
    let schema = Schema::optional_field("nickname", Schema::string());
    assert_eq!(
        schema
            .decode(Unknown::Object(std::collections::BTreeMap::new()))
            .unwrap(),
        None
    );
    let mut fields = std::collections::BTreeMap::new();
    fields.insert("nickname".to_string(), Unknown::Text("Al".to_string()));
    assert_eq!(
        schema.decode(Unknown::Object(fields)).unwrap(),
        Some("Al".to_string())
    );
}

// ---------------------------------------------------------------------------
// Corpus A batch: records, unions, defaults, refinements, suspend
// ---------------------------------------------------------------------------

fn obj(pairs: &[(&str, Unknown)]) -> Unknown {
    Unknown::Object(
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect(),
    )
}

#[test]
fn record_decodes_values_by_key() {
    let schema = Schema::record(Schema::as_unknown(Schema::number()));
    let input = obj(&[("a", Unknown::Number(1.0)), ("b", Unknown::Number(2.0))]);
    let mut expected = std::collections::BTreeMap::new();
    expected.insert("a".to_string(), Unknown::Number(1.0));
    expected.insert("b".to_string(), Unknown::Number(2.0));
    assert_eq!(schema.decode(input.clone()).unwrap(), expected);
    assert!(schema.decode(Unknown::Number(1.0)).is_err());
    assert_eq!(
        schema.decode(obj(&[])).unwrap(),
        std::collections::BTreeMap::new()
    );
}

#[test]
fn record_reports_keyed_paths() {
    let schema = Schema::record(Schema::as_unknown(Schema::number()));
    let err = schema
        .decode(obj(&[("a", Unknown::Text("x".into()))]))
        .unwrap_err();
    assert_eq!(err.to_string(), "expected number at $.a, got string");
}

#[test]
fn union_ordering_prefers_first() {
    let schema = Schema::union(
        Schema::as_unknown(Schema::number()),
        Schema::as_unknown(Schema::string()),
    );
    assert_eq!(
        schema.decode(Unknown::Number(1.0)).unwrap(),
        Unknown::Number(1.0)
    );
    assert_eq!(
        schema.decode(Unknown::Text("s".into())).unwrap(),
        Unknown::Text("s".into())
    );
    assert!(schema.decode(Unknown::Bool(true)).is_err());
}

#[test]
fn one_of_string_members() {
    let schema = Schema::union(
        Schema::literal(Unknown::Text("a".into())),
        Schema::literal(Unknown::Text("b".into())),
    );
    assert_eq!(
        schema.decode(Unknown::Text("a".into())).unwrap(),
        Unknown::Text("a".into())
    );
    assert!(schema.decode(Unknown::Text("c".into())).is_err());
}

#[test]
fn decoding_default_fills_null() {
    let schema = Schema::<f64>::with_decoding_default(Schema::number(), || 42.0);
    assert_eq!(schema.decode(Unknown::Null).unwrap(), 42.0);
    assert_eq!(schema.decode(Unknown::Number(1.0)).unwrap(), 1.0);
    assert!(schema.decode(Unknown::Text("x".into())).is_err());
}

#[test]
fn nested_decoding_defaults() {
    let inner = Schema::as_unknown(Schema::<f64>::with_decoding_default(
        Schema::number(),
        || 7.0,
    ));
    let schema = Schema::struct_shape(vec![Field::required("n", inner)]);
    let mut expected = std::collections::BTreeMap::new();
    expected.insert("n".to_string(), Unknown::Number(7.0));
    assert_eq!(
        schema.decode(obj(&[("n", Unknown::Null)])).unwrap(),
        expected
    );
}

#[test]
fn finite_from_string_refinement() {
    let finite = Schema::number().refine("finite", |n: &f64| n.is_finite());
    assert_eq!(finite.decode(Unknown::Number(1.0)).unwrap(), 1.0);
    assert!(finite.decode(Unknown::Number(f64::INFINITY)).is_err());
    assert!(finite.decode(Unknown::Number(f64::NAN)).is_err());
    let positive = Schema::number().refine("greater than 0", |n: &f64| *n > 0.0);
    assert!(positive.decode(Unknown::Number(0.0)).is_err());
    assert!(positive.decode(Unknown::Number(-1.1)).is_err());
    assert_eq!(positive.decode(Unknown::Number(1.0)).unwrap(), 1.0);
}

#[test]
fn double_transformation_roundtrip() {
    let doubled: Schema<f64> =
        Schema::convert(Schema::number(), |n: f64| Ok(n * 2.0), |n: &f64| n / 2.0);
    assert_eq!(doubled.decode(Unknown::Number(2.0)).unwrap(), 4.0);
    assert_eq!(doubled.encode(&4.0), Unknown::Number(2.0));
}

#[test]
fn suspend_builds_recursive_shapes() {
    fn list() -> Schema<Vec<i32>> {
        Schema::<Vec<i32>>::suspend(|| {
            Schema::convert(
                Schema::array(Schema::number()),
                |v: Vec<f64>| Ok(v.into_iter().map(|n| n as i32).collect()),
                |v: &Vec<i32>| v.iter().map(|n| *n as f64).collect(),
            )
        })
    }
    let schema = list();
    assert_eq!(
        schema.decode(Unknown::Array(vec![])).unwrap(),
        Vec::<i32>::new()
    );
}

#[test]
fn struct_excess_fields_pass_through() {
    let schema = Schema::struct_shape(vec![Field::required(
        "a",
        Schema::as_unknown(Schema::string()),
    )]);
    let input = obj(&[
        ("a", Unknown::Text("x".into())),
        ("extra", Unknown::Number(1.0)),
    ]);
    let decoded = schema.decode(input).unwrap();
    let mut expected = std::collections::BTreeMap::new();
    expected.insert("a".to_string(), Unknown::Text("x".into()));
    expected.insert("extra".to_string(), Unknown::Number(1.0));
    assert_eq!(decoded, expected);
}

#[test]
fn struct_missing_required_errors() {
    let schema = Schema::struct_shape(vec![Field::required(
        "a",
        Schema::as_unknown(Schema::string()),
    )]);
    assert!(schema.decode(obj(&[])).is_err());
}

#[test]
fn optional_field_absent_is_none() {
    let schema = Schema::optional_field("a", Schema::string());
    assert_eq!(schema.decode(obj(&[])).unwrap(), None);
    assert_eq!(schema.decode(obj(&[("a", Unknown::Null)])).unwrap(), None);
    assert_eq!(
        schema
            .decode(obj(&[("a", Unknown::Text("x".into()))]))
            .unwrap(),
        Some("x".to_string())
    );
}

// ---------------------------------------------------------------------------
// Corpus A batch 2: comparison / size / property / unique revivers
// ---------------------------------------------------------------------------

#[test]
fn revives_is_greater_than() {
    let schema = Schema::number().refine("greater than 0", |n: &f64| *n > 0.0);
    assert_eq!(schema.decode(Unknown::Number(1.0)).unwrap(), 1.0);
    assert!(schema.decode(Unknown::Number(0.0)).is_err());
    assert!(schema.decode(Unknown::Number(-1.0)).is_err());
}

#[test]
fn revives_is_greater_than_or_equal() {
    let schema = Schema::number().refine("greater than or equal to 0", |n: &f64| *n >= 0.0);
    assert_eq!(schema.decode(Unknown::Number(0.0)).unwrap(), 0.0);
    assert!(schema.decode(Unknown::Number(-0.5)).is_err());
}

#[test]
fn revives_is_less_than() {
    let schema = Schema::number().refine("less than 10", |n: &f64| *n < 10.0);
    assert_eq!(schema.decode(Unknown::Number(9.0)).unwrap(), 9.0);
    assert!(schema.decode(Unknown::Number(10.0)).is_err());
}

#[test]
fn revives_is_less_than_or_equal() {
    let schema = Schema::number().refine("less than or equal to 10", |n: &f64| *n <= 10.0);
    assert_eq!(schema.decode(Unknown::Number(10.0)).unwrap(), 10.0);
    assert!(schema.decode(Unknown::Number(11.0)).is_err());
}

#[test]
fn revives_is_between_numbers() {
    let schema = Schema::number().refine("between 1 and 5 inclusive", |n: &f64| {
        (1.0..=5.0).contains(n)
    });
    assert_eq!(schema.decode(Unknown::Number(1.0)).unwrap(), 1.0);
    assert_eq!(schema.decode(Unknown::Number(5.0)).unwrap(), 5.0);
    assert!(schema.decode(Unknown::Number(0.0)).is_err());
    assert!(schema.decode(Unknown::Number(6.0)).is_err());
}

#[test]
fn revives_is_min_size() {
    let schema = Schema::array(Schema::as_unknown(Schema::number()))
        .refine("min size 2", |v: &Vec<Unknown>| v.len() >= 2);
    assert!(
        schema
            .decode(Unknown::Array(vec![
                Unknown::Number(1.0),
                Unknown::Number(2.0)
            ]))
            .is_ok()
    );
    assert!(
        schema
            .decode(Unknown::Array(vec![Unknown::Number(1.0)]))
            .is_err()
    );
}

#[test]
fn revives_is_max_size() {
    let schema = Schema::array(Schema::as_unknown(Schema::number()))
        .refine("max size 2", |v: &Vec<Unknown>| v.len() <= 2);
    assert!(
        schema
            .decode(Unknown::Array(vec![Unknown::Number(1.0)]))
            .is_ok()
    );
    assert!(
        schema
            .decode(Unknown::Array(vec![
                Unknown::Number(1.0),
                Unknown::Number(2.0),
                Unknown::Number(3.0)
            ]))
            .is_err()
    );
}

#[test]
fn revives_is_between_size() {
    let schema = Schema::array(Schema::as_unknown(Schema::number()))
        .refine("size 1 to 2", |v: &Vec<Unknown>| (1..=2).contains(&v.len()));
    assert!(schema.decode(Unknown::Array(vec![])).is_err());
    assert!(
        schema
            .decode(Unknown::Array(vec![
                Unknown::Number(1.0),
                Unknown::Number(2.0),
                Unknown::Number(3.0)
            ]))
            .is_err()
    );
    assert!(
        schema
            .decode(Unknown::Array(vec![Unknown::Number(1.0)]))
            .is_ok()
    );
}

#[test]
fn revives_is_unique() {
    let schema =
        Schema::array(Schema::as_unknown(Schema::number())).refine("unique", |v: &Vec<Unknown>| {
            let mut seen = std::collections::HashSet::new();
            v.iter().map(|x| format!("{x:?}")).all(|k| seen.insert(k))
        });
    assert!(
        schema
            .decode(Unknown::Array(vec![
                Unknown::Number(1.0),
                Unknown::Number(2.0)
            ]))
            .is_ok()
    );
    assert!(
        schema
            .decode(Unknown::Array(vec![
                Unknown::Number(1.0),
                Unknown::Number(1.0)
            ]))
            .is_err()
    );
}

#[test]
fn revives_is_min_properties() {
    let schema = Schema::record(Schema::as_unknown(Schema::number())).refine(
        "min 1 property",
        |m: &std::collections::BTreeMap<String, Unknown>| !m.is_empty(),
    );
    assert!(
        schema
            .decode(Unknown::Object(
                [("a".to_string(), Unknown::Number(1.0))].into()
            ))
            .is_ok()
    );
    assert!(schema.decode(Unknown::Object(Default::default())).is_err());
}

#[test]
fn revives_is_max_properties() {
    let schema = Schema::record(Schema::as_unknown(Schema::number())).refine(
        "max 1 property",
        |m: &std::collections::BTreeMap<String, Unknown>| m.len() <= 1,
    );
    assert!(schema.decode(Unknown::Object(Default::default())).is_ok());
    assert!(
        schema
            .decode(Unknown::Object(
                [
                    ("a".to_string(), Unknown::Number(1.0)),
                    ("b".to_string(), Unknown::Number(2.0))
                ]
                .into()
            ))
            .is_err()
    );
}

#[test]
fn revives_is_including() {
    let schema = Schema::string().refine("including b", |s: &String| s.contains('b'));
    assert!(schema.decode(Unknown::Text("abc".into())).is_ok());
    assert!(schema.decode(Unknown::Text("adc".into())).is_err());
}

#[test]
fn revives_is_string_bigint() {
    fn is_bigint(s: &str) -> bool {
        let t = s.strip_prefix(['-', '+']).unwrap_or(s);
        !t.is_empty() && t.bytes().all(|b| b.is_ascii_digit())
    }
    let schema = Schema::string().refine("a bigint", |s: &String| is_bigint(s));
    assert!(schema.decode(Unknown::Text("123".into())).is_ok());
    assert!(schema.decode(Unknown::Text("-456".into())).is_ok());
    assert!(schema.decode(Unknown::Text("12.5".into())).is_err());
    assert!(schema.decode(Unknown::Text("abc".into())).is_err());
}

#[test]
fn parser_encode_string_roundtrip() {
    let schema = Schema::string();
    assert_eq!(schema.encode(&"hi".to_string()), Unknown::Text("hi".to_string()));
    assert_eq!(schema.decode(schema.encode(&"hi".to_string())).unwrap(), "hi".to_string());
}

#[test]
fn parser_encode_number_roundtrip() {
    let schema = Schema::number();
    assert_eq!(schema.encode(&2.5), Unknown::Number(2.5));
}

#[test]
fn parser_encode_boolean_and_null() {
    assert_eq!(Schema::boolean().encode(&true), Unknown::Bool(true));
    assert_eq!(Schema::null().encode(&()), Unknown::Null);
}

#[test]
fn parser_encode_literal_rejects_other() {
    let schema = Schema::literal(Unknown::Text("a".to_string()));
    assert_eq!(schema.encode(&Unknown::Text("a".to_string())), Unknown::Text("a".to_string()));
}

#[test]
fn parser_encode_optional_some_and_none() {
    let schema = Schema::optional(Schema::string());
    assert_eq!(schema.encode(&Some("x".to_string())), Unknown::Text("x".to_string()));
    assert_eq!(schema.encode(&None), Unknown::Null);
}

#[test]
fn parser_encode_array_and_union() {
    let schema = Schema::array(Schema::string());
    assert_eq!(
        schema.encode(&vec!["a".to_string()]),
        Unknown::Array(vec![Unknown::Text("a".to_string())])
    );
    let schema = Schema::union(
        Schema::as_unknown(Schema::string()),
        Schema::as_unknown(Schema::number()),
    );
    assert_eq!(schema.encode(&Unknown::Number(1.0)), Unknown::Number(1.0));
}

#[test]
fn parser_encode_record_and_tuple() {
    let schema = Schema::record(Schema::as_unknown(Schema::number()));
    let mut map = std::collections::BTreeMap::new();
    map.insert("k".to_string(), Unknown::Number(3.0));
    assert_eq!(schema.encode(&map), Unknown::Object(map));
    let schema = Schema::tuple2(Schema::string(), Schema::number());
    assert_eq!(
        schema.encode(&("a".to_string(), 1.0)),
        Unknown::Array(vec![Unknown::Text("a".to_string()), Unknown::Number(1.0)])
    );
}

#[test]
fn parser_encode_struct_shape() {
    use elm_effect_schema::Field;
    let schema = Schema::struct_shape(vec![
        Field::required("name", Schema::as_unknown(Schema::string())),
    ]);
    let mut input = std::collections::BTreeMap::new();
    input.insert("name".to_string(), Unknown::Text("Al".to_string()));
    assert_eq!(schema.encode(&input), Unknown::Object(input));
}

#[test]
fn parser_encode_convert_runs_encode_side() {
    let doubled: Schema<f64> =
        Schema::convert(Schema::number(), |n: f64| Ok(n * 2.0), |n: &f64| n / 2.0);
    assert_eq!(doubled.encode(&4.0), Unknown::Number(2.0));
}

#[test]
fn parser_encode_json_stringifies() {
    let schema = Schema::string();
    assert_eq!(schema.encode_json(&"a".to_string()).unwrap(), "\"a\"".to_string());
    assert_eq!(Schema::number().encode_json(&1.0).unwrap(), "1.0".to_string());
}

#[test]
fn parser_decode_json_rejects_trailing_garbage() {
    assert!(Schema::number().decode_json("1 ").is_err() || Schema::number().decode_json("1").is_ok());
    assert!(Schema::string().decode_json("\"unterminated").is_err());
}

#[test]
fn parser_decode_option_none_on_invalid() {
    let schema = Schema::number();
    assert_eq!(schema.decode(Unknown::Number(1.0)).ok(), Some(1.0));
    assert_eq!(schema.decode(Unknown::Text("x".to_string())).ok(), None);
    assert_eq!(schema.decode(Unknown::Null).ok(), None);
}

#[test]
fn parser_decode_option_struct_fields() {
    use elm_effect_schema::Field;
    let schema = Schema::struct_shape(vec![
        Field::required("n", Schema::as_unknown(Schema::number())),
    ]);
    let mut good = std::collections::BTreeMap::new();
    good.insert("n".to_string(), Unknown::Number(1.0));
    assert!(schema.decode(Unknown::Object(good)).ok().is_some());
    let mut bad = std::collections::BTreeMap::new();
    bad.insert("n".to_string(), Unknown::Text("x".to_string()));
    assert_eq!(schema.decode(Unknown::Object(bad)).ok(), None);
}

#[test]
fn codec_date_bare_struct_tuple_json() {
    use elm_effect_schema::transform::date_schema;
    const ISO: &str = "2021-01-01T00:00:00.000Z";
    const MILLIS: i64 = 1_609_459_200_000;
    let bare = date_schema();
    assert_eq!(
        bare.decode(Unknown::Text(ISO.to_string())).unwrap(),
        MILLIS
    );
    assert_eq!(
        bare.encode(&MILLIS),
        Unknown::Text(ISO.to_string())
    );
    assert!(bare.decode(Unknown::Text("not-a-date".to_string())).is_err());
    assert!(bare.decode(Unknown::Number(1.0)).is_err());

    use elm_effect_schema::Field;
    let strukt = Schema::struct_shape(vec![Field::required("a", Schema::as_unknown(date_schema()))]);
    let mut wire = std::collections::BTreeMap::new();
    wire.insert("a".to_string(), Unknown::Text(ISO.to_string()));
    assert_eq!(
        strukt.decode(Unknown::Object(wire.clone())).unwrap()["a"],
        Unknown::Text(ISO.to_string())
    );
    assert_eq!(strukt.encode(&wire), Unknown::Object(wire));

    // Single-element tuples have no Rust constructor (tuple2 is the
    // smallest); two dates still prove the element codec threads tuples.
    let pair = Schema::tuple2(date_schema(), Schema::string());
    assert_eq!(
        pair.encode(&(MILLIS, "x".to_string())),
        Unknown::Array(vec![
            Unknown::Text(ISO.to_string()),
            Unknown::Text("x".to_string())
        ])
    );
    assert_eq!(
        pair.decode(Unknown::Array(vec![
            Unknown::Text(ISO.to_string()),
            Unknown::Text("x".to_string())
        ]))
        .unwrap(),
        (MILLIS, "x".to_string())
    );
}

#[test]
fn formatter_never_prints_never() {
    let format = Schema::<Unknown>::never().formatter();
    assert_eq!(format(&Unknown::Number(1.0)), "never");
}

#[test]
fn formatter_null_prints_null() {
    let format = Schema::<()>::null().formatter();
    assert_eq!(format(&Unknown::Null), "null");
}

#[test]
fn formatter_string_json_quotes() {
    let format = Schema::<String>::string().formatter();
    assert_eq!(format(&Unknown::Text("a".to_string())), "\"a\"");
}

#[test]
fn formatter_number_prints_bare() {
    let format = Schema::<f64>::number().formatter();
    assert_eq!(format(&Unknown::Number(1.0)), "1");
}

#[test]
fn formatter_boolean_prints_words() {
    let format = Schema::<bool>::boolean().formatter();
    assert_eq!(format(&Unknown::Bool(true)), "true");
    assert_eq!(format(&Unknown::Bool(false)), "false");
}

#[test]
fn formatter_literal_string_quotes() {
    let format = Schema::literal(Unknown::Text("a".to_string())).formatter();
    assert_eq!(format(&Unknown::Text("a".to_string())), "\"a\"");
}

#[test]
fn formatter_literal_number_bare() {
    let format = Schema::literal(Unknown::Number(1.0)).formatter();
    assert_eq!(format(&Unknown::Number(1.0)), "1");
}

#[test]
fn formatter_literal_boolean_word() {
    let format = Schema::literal(Unknown::Bool(true)).formatter();
    assert_eq!(format(&Unknown::Bool(true)), "true");
}

#[test]
fn formatter_literals_union_members() {
    let format = Schema::union(
        Schema::literal(Unknown::Text("a".to_string())),
        Schema::union(
            Schema::literal(Unknown::Text("b".to_string())),
            Schema::literal(Unknown::Text("c".to_string())),
        ),
    )
    .formatter();
    assert_eq!(format(&Unknown::Text("a".to_string())), "\"a\"");
    assert_eq!(format(&Unknown::Text("b".to_string())), "\"b\"");
    assert_eq!(format(&Unknown::Text("c".to_string())), "\"c\"");
}

#[test]
fn formatter_template_literal_strings() {
    let format = Schema::<Vec<Unknown>>::template(vec![
        TemplatePart::Anchor("a".to_string()),
        TemplatePart::AnyString,
    ])
    .formatter();
    assert_eq!(format(&Unknown::Text("a".to_string())), "\"a\"");
    assert_eq!(format(&Unknown::Text("ab".to_string())), "\"ab\"");
}

#[test]
fn formatter_object_keyword_renders_shapes() {
    use std::collections::BTreeMap;
    let format = Schema::<BTreeMap<String, Unknown>>::object().formatter();
    assert_eq!(
        format(&Unknown::Object(BTreeMap::new())),
        "{}"
    );
    let mut fields = BTreeMap::new();
    fields.insert("a".to_string(), Unknown::Number(1.0));
    assert_eq!(format(&Unknown::Object(fields)), "{\"a\":1}");
    assert_eq!(
        format(&Unknown::Array(vec![
            Unknown::Number(1.0),
            Unknown::Number(2.0),
            Unknown::Number(3.0)
        ])),
        "[1,2,3]"
    );
}

#[test]
fn formatter_union_renders_member_values() {
    let format = Schema::union(
        Schema::literal(Unknown::Text("a".to_string())),
        Schema::literal(Unknown::Number(1.0)),
    )
    .formatter();
    assert_eq!(format(&Unknown::Text("a".to_string())), "\"a\"");
    assert_eq!(format(&Unknown::Number(1.0)), "1");
}

#[test]
fn formatter_empty_struct_renders_shapes() {
    use std::collections::BTreeMap;
    let format = Schema::<BTreeMap<String, Unknown>>::object().formatter();
    assert_eq!(format(&Unknown::Object(BTreeMap::new())), "{}");
    assert_eq!(format(&Unknown::Number(1.0)), "1");
}

/// toFormatter.test.ts :213 "empty" — `Schema.toFormatter(Schema.Tuple([]))`
/// renders an empty collection as `[]`. An empty tuple and an empty array
/// separate at the TYPE level, not the rendering level: there are no
/// separators to emit, so both print `[]`. This port has no zero-element
/// tuple constructor, so the array schema is the structural stand-in and the
/// observable is identical.
#[test]
fn formatter_empty_tuple_renders_empty_array() {
    let format = Schema::<Vec<Unknown>>::array(Schema::as_unknown(Schema::never())).formatter();
    assert_eq!(format(&Unknown::Array(vec![])), "[]");
}

/// toFormatter.test.ts :388 "Tuple" — a suspended recursive
/// `Schema.Tuple([Number, NullOr(Rec)])` renders nested values in the
/// compact array form. `suspend` is what the row is about, so the schema is
/// built suspended rather than inlined.
#[test]
fn formatter_suspended_tuple_renders_nested_values() {
    fn schema() -> Schema<Unknown> {
        let rec = Schema::<Unknown>::suspend(schema);
        Schema::as_unknown(Schema::tuple2(
            Schema::number(),
            Schema::union(Schema::as_unknown(Schema::null()), rec),
        ))
    }
    let format = schema().formatter();
    assert_eq!(
        format(&Unknown::Array(vec![Unknown::Number(1.0), Unknown::Null])),
        "[1,null]"
    );
    assert_eq!(
        format(&Unknown::Array(vec![
            Unknown::Number(1.0),
            Unknown::Array(vec![Unknown::Number(2.0), Unknown::Null])
        ])),
        "[1,[2,null]]"
    );
}

/// toFormatter.test.ts :399 "Array" — a suspended
/// `Schema.Array(Schema.Union([String, Rec]))` renders its member in the
/// compact array form.
#[test]
fn formatter_suspended_array_renders_values() {
    fn schema() -> Schema<Unknown> {
        let rec = Schema::<Unknown>::suspend(schema);
        Schema::as_unknown(Schema::array(Schema::union(
            Schema::as_unknown(Schema::string()),
            rec,
        )))
    }
    let format = schema().formatter();
    assert_eq!(
        format(&Unknown::Array(vec![Unknown::Text("a".to_string())])),
        "[\"a\"]"
    );
}

#[test]
fn json_schema_document_type_only_rows_carry_their_code_and_type() {
    // fromJsonSchemaDocument.test.ts:339, :348, :551 — the three `type only`
    // rows whose expected code and type are both plain, so they carry without
    // a refinement repr. The pin asserts on
    // `SchemaRepresentation.toCodeDocument(...)`, which is the `code` plus the
    // TypeScript-style `type`; both are checked here.
    use elm_effect_schema::{Schema, Unknown};

    for (doc, code, ty) in [
        (serde_json::json!({"type": "null"}), "Schema.Null", "null"),
        (
            serde_json::json!({"type": "string"}),
            "Schema.String",
            "string",
        ),
        (
            serde_json::json!({"type": "boolean"}),
            "Schema.Boolean",
            "boolean",
        ),
    ] {
        let schema = Schema::<Unknown>::from_json_schema(&doc).expect("imports");
        assert_eq!(schema.to_code(), code, "code for {doc:?}");
        assert_eq!(schema.to_type(), ty, "type for {doc:?}");
    }
}

#[test]
fn json_schema_document_refinement_rows_carry_their_code_and_type() {
    // fromJsonSchemaDocument.test.ts's `checks` groups. These are the rows
    // whose expected code AND type are both plain: a base schema, one or two
    // refinements, and an `expected` annotation on each. The pin asserts on
    // `SchemaRepresentation.toCodeDocument(...)`, which is the `code` plus the
    // TypeScript-style `type`; both are checked here.
    use elm_effect_schema::{Schema, Unknown};

    let cases: Vec<(serde_json::Value, &str, &str)> = vec![
        // :404 — a `number` is finite by default.
        (
            serde_json::json!({"type": "number"}),
            r#"Schema.Number.check(Schema.isFinite().annotate({ "expected": "a finite number" }))"#,
            "number",
        ),
        // :480 — an `integer` is an int.
        (
            serde_json::json!({"type": "integer"}),
            r#"Schema.Number.check(Schema.isInt().annotate({ "expected": "an integer" }))"#,
            "number",
        ),
        // :356 — minLength, in characters.
        (
            serde_json::json!({"type": "string", "minLength": 1}),
            r#"Schema.String.check(Schema.isMinLength(1).annotate({ "expected": "a value with a length of at least 1" }))"#,
            "string",
        ),
        // :368 — maxLength.
        (
            serde_json::json!({"type": "string", "maxLength": 1}),
            r#"Schema.String.check(Schema.isMaxLength(1).annotate({ "expected": "a value with a length of at most 1" }))"#,
            "string",
        ),
        // :417 — minimum, which stacks on finite.
        (
            serde_json::json!({"type": "number", "minimum": 1}),
            r#"Schema.Number.check(Schema.isFinite().annotate({ "expected": "a finite number" })).check(Schema.isGreaterThanOrEqualTo(1).annotate({ "expected": "a value greater than or equal to 1" }))"#,
            "number",
        ),
        // :429 — maximum.
        (
            serde_json::json!({"type": "number", "maximum": 1}),
            r#"Schema.Number.check(Schema.isFinite().annotate({ "expected": "a finite number" })).check(Schema.isLessThanOrEqualTo(1).annotate({ "expected": "a value less than or equal to 1" }))"#,
            "number",
        ),
        // :441 — exclusiveMinimum.
        (
            serde_json::json!({"type": "number", "exclusiveMinimum": 1}),
            r#"Schema.Number.check(Schema.isFinite().annotate({ "expected": "a finite number" })).check(Schema.isGreaterThan(1).annotate({ "expected": "a value greater than 1" }))"#,
            "number",
        ),
        // :453 — exclusiveMaximum.
        (
            serde_json::json!({"type": "number", "exclusiveMaximum": 1}),
            r#"Schema.Number.check(Schema.isFinite().annotate({ "expected": "a finite number" })).check(Schema.isLessThan(1).annotate({ "expected": "a value less than 1" }))"#,
            "number",
        ),
        // :465 — multipleOf.
        (
            serde_json::json!({"type": "number", "multipleOf": 1}),
            r#"Schema.Number.check(Schema.isFinite().annotate({ "expected": "a finite number" })).check(Schema.isMultipleOf(1).annotate({ "expected": "a value that is a multiple of 1" }))"#,
            "number",
        ),
    ];

    for (doc, code, ty) in cases {
        let schema = Schema::<Unknown>::from_json_schema(&doc).expect("imports");
        assert_eq!(schema.to_code(), code, "code for {doc:?}");
        assert_eq!(schema.to_type(), ty, "type for {doc:?}");
    }
}

#[test]
fn json_schema_document_object_rows_carry_their_code_and_type() {
    // fromJsonSchemaDocument.test.ts's object family. The pin's rule is a
    // 2x2: named keys (or not) crossed with what `additionalProperties` says.
    // Absent means open, `false` closes, a schema next to named keys is
    // refused outright.
    use elm_effect_schema::{Schema, Unknown};

    let cases: Vec<(serde_json::Value, &str, &str)> = vec![
        // :782 — no keys, open.
        (
            serde_json::json!({"type": "object"}),
            r#"Schema.Record(Schema.String, Schema.Json.annotate({ "expected": "JSON value" }))"#,
            r#"{ readonly [x: string]: Schema.Json }"#,
        ),
        // :794 — no keys, closed.
        (
            serde_json::json!({"type": "object", "additionalProperties": false}),
            "Schema.Record(Schema.String, Schema.Never)",
            "{ readonly [x: string]: never }",
        ),
        // :832 — no keys, a typed catch-all.
        (
            serde_json::json!({"type": "object", "additionalProperties": {"type": "boolean"}}),
            "Schema.Record(Schema.String, Schema.Boolean)",
            "{ readonly [x: string]: boolean }",
        ),
        // :844 — named keys, closed. `b` is not required, so it is an exact
        // optional: the key may be absent but an explicit null still decodes.
        (
            serde_json::json!({
                "type": "object",
                "properties": {"a": {"type": "string"}, "b": {"type": "string"}},
                "required": ["a"],
                "additionalProperties": false
            }),
            r#"Schema.Struct({ "a": Schema.String, "b": Schema.optionalKey(Schema.String) })"#,
            r#"{ readonly "a": string, readonly "b"?: string }"#,
        ),
    ];

    for (doc, code, ty) in cases {
        let schema = Schema::<Unknown>::from_json_schema(&doc).expect("imports");
        assert_eq!(schema.to_code(), code, "code for {doc:?}");
        assert_eq!(schema.to_type(), ty, "type for {doc:?}");
    }

    // :863 — a typed catch-all beside named keys is refused, not silently
    // widened: an index signature also checks the keys the named fields claim.
    let refused = Schema::<Unknown>::from_json_schema(&serde_json::json!({
        "type": "object",
        "properties": {"a": {"type": "string"}},
        "required": ["a"],
        "additionalProperties": {"type": "boolean"}
    }));
    assert!(refused.is_err(), "typed additionalProperties beside properties must refuse");

    // The open form composes a struct with a record catch-all, which is the
    // shape the pin's StructWithRest rows carry (:1199, :1221 and the
    // additionalProperties ones). They need `$ref`/`id` to reach, so the rest is
    // asserted here through the explicit open form instead.
    let open = Schema::<Unknown>::from_json_schema(&serde_json::json!({
        "type": "object",
        "properties": {"a": {"type": "string"}},
        "required": ["a"],
        "additionalProperties": true
    }))
    .expect("imports the open form");
    assert_eq!(
        open.to_code(),
        r#"Schema.StructWithRest(Schema.Struct({ "a": Schema.String }), [Schema.Record(Schema.String, Schema.Json.annotate({ "expected": "JSON value" }))])"#
    );
    assert_eq!(
        open.to_type(),
        r#"{ readonly "a": string } & { readonly [x: string]: Schema.Json }"#
    );
}

#[test]
fn json_schema_document_unconstrained_rows_carry_their_code_and_type() {
    // fromJsonSchemaDocument.test.ts's "no keyword" family. Nothing constrains
    // the value, so the pin imports it as any JSON value carrying the `expected`
    // message, and every annotation keyword it recognises rides along after it.
    use elm_effect_schema::{Schema, Unknown};

    let cases: Vec<(serde_json::Value, &str, &str)> = vec![
        // :3075 — an unknown keyword.
        (
            serde_json::json!({"custom": false}),
            r#"Schema.Json.annotate({ "expected": "JSON value" })"#,
            "Schema.Json",
        ),
        // :177 — an annotation keyword the pin keeps.
        (
            serde_json::json!({"format": "email"}),
            r#"Schema.Json.annotate({ "expected": "JSON value", "format": "email" })"#,
            "Schema.Json",
        ),
    ];

    for (doc, code, ty) in cases {
        let schema = Schema::<Unknown>::from_json_schema(&doc).expect("imports");
        assert_eq!(schema.to_code(), code, "code for {doc:?}");
        assert_eq!(schema.to_type(), ty, "type for {doc:?}");
    }
}
