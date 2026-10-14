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
    // The default is wire-shaped, so it goes through the schema on the way in.
    let schema = Schema::with_decoding_default(Schema::number(), || Ok(Unknown::Number(42.0)));
    assert_eq!(schema.decode(Unknown::Null).unwrap(), 42.0);
    assert_eq!(schema.decode(Unknown::Number(1.0)).unwrap(), 1.0);
    assert!(schema.decode(Unknown::Text("x".into())).is_err());

    // The decoded spelling skips that step and is used as it is.
    let typed = Schema::with_decoding_default_type(Schema::number(), || Ok(42.0));
    assert_eq!(typed.decode(Unknown::Null).unwrap(), 42.0);
}

#[test]
fn nested_decoding_defaults() {
    let inner = Schema::as_unknown(Schema::with_decoding_default(Schema::number(), || {
        Ok(Unknown::Number(7.0))
    }));
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
fn struct_excess_fields_are_stripped_by_default() {
    // This test used to assert the opposite. The pin's decoder default is
    // `onExcessProperty: "ignore"` — "Closed objects normally strip excess
    // properties by default; pass `onExcessProperty: \"error\"` to reject them"
    // (SchemaRepresentation.ts:2968, and Schema.ts:15411 says the same of the
    // decoder) — so a closed struct keeps its declared fields and drops the
    // rest. `struct_with_rest` is the shape that absorbs the rest, and the row
    // that caught this is Schema.test.ts:666, whose explicit
    // `onExcessProperty: "ignore"` names the default.
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
    assert_eq!(
        schema.encode(&"hi".to_string()),
        Unknown::Text("hi".to_string())
    );
    assert_eq!(
        schema.decode(schema.encode(&"hi".to_string())).unwrap(),
        "hi".to_string()
    );
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
    assert_eq!(
        schema.encode(&Unknown::Text("a".to_string())),
        Unknown::Text("a".to_string())
    );
}

#[test]
fn parser_encode_optional_some_and_none() {
    let schema = Schema::optional(Schema::string());
    assert_eq!(
        schema.encode(&Some("x".to_string())),
        Unknown::Text("x".to_string())
    );
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
    let schema = Schema::struct_shape(vec![Field::required(
        "name",
        Schema::as_unknown(Schema::string()),
    )]);
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
    assert_eq!(
        schema.encode_json(&"a".to_string()).unwrap(),
        "\"a\"".to_string()
    );
    assert_eq!(
        Schema::number().encode_json(&1.0).unwrap(),
        "1.0".to_string()
    );
}

#[test]
fn parser_decode_json_rejects_trailing_garbage() {
    assert!(
        Schema::number().decode_json("1 ").is_err() || Schema::number().decode_json("1").is_ok()
    );
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
    let schema = Schema::struct_shape(vec![Field::required(
        "n",
        Schema::as_unknown(Schema::number()),
    )]);
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
    assert_eq!(bare.decode(Unknown::Text(ISO.to_string())).unwrap(), MILLIS);
    assert_eq!(bare.encode(&MILLIS), Unknown::Text(ISO.to_string()));
    assert!(
        bare.decode(Unknown::Text("not-a-date".to_string()))
            .is_err()
    );
    assert!(bare.decode(Unknown::Number(1.0)).is_err());

    use elm_effect_schema::Field;
    let strukt = Schema::struct_shape(vec![Field::required(
        "a",
        Schema::as_unknown(date_schema()),
    )]);
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
    assert_eq!(format(&Unknown::Object(BTreeMap::new())), "{}");
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
            r#"Schema.String.check(Schema.isMaxCodePoints(1).annotate({ "expected": "a string with at most 1 code points" }))"#,
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
    assert!(
        refused.is_err(),
        "typed additionalProperties beside properties must refuse"
    );

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

#[test]
fn json_schema_document_prefix_items_rows_carry_their_code_and_type() {
    // fromJsonSchemaDocument.test.ts's prefixItems family. `minItems` marks the
    // leading positions required, `maxItems` decides whether any position is
    // open, and `items` names the open element. Every combination below is a
    // row from the file.
    use elm_effect_schema::{Schema, Unknown};

    let json_rest = r#"Schema.Json.annotate({ "expected": "JSON value" })"#;
    let cases: Vec<(serde_json::Value, String, &str)> = vec![
        // :584 — maxItems below the prefix length: the rest stays, and a
        // max-length check rides along because the rest cannot hold every
        // declared position the tuple otherwise would.
        (
            serde_json::json!({
                "type": "array",
                "prefixItems": [{"type": "string"}, {"type": "number"}],
                "maxItems": 1
            }),
            format!(
                r#"Schema.TupleWithRest(Schema.Tuple([Schema.optionalKey(Schema.String), Schema.optionalKey(Schema.Number.check(Schema.isFinite().annotate({{ "expected": "a finite number" }})))]), [{json_rest}]).check(Schema.isMaxLength(1).annotate({{ "expected": "a value with a length of at most 1" }}))"#
            ),
            r#"readonly [(string)?, (number)?, ...Array<Schema.Json>]"#,
        ),
        // :602 — maxItems above the prefix length: rest, no check.
        (
            serde_json::json!({
                "type": "array",
                "prefixItems": [{"type": "string"}],
                "maxItems": 2
            }),
            format!(
                r#"Schema.TupleWithRest(Schema.Tuple([Schema.optionalKey(Schema.String)]), [{json_rest}]).check(Schema.isMaxLength(2).annotate({{ "expected": "a value with a length of at most 2" }}))"#
            ),
            r#"readonly [(string)?, ...Array<Schema.Json>]"#,
        ),
        // :620 — `items: false` closes the tuple, so the rest is dropped.
        (
            serde_json::json!({
                "type": "array",
                "prefixItems": [{"type": "string"}],
                "items": false,
                "maxItems": 2
            }),
            "Schema.Tuple([Schema.optionalKey(Schema.String)])".to_string(),
            "readonly [(string)?]",
        ),
        // :634 — maxItems equal to the prefix length also closes it.
        (
            serde_json::json!({
                "type": "array",
                "prefixItems": [{"type": "string"}],
                "maxItems": 1
            }),
            "Schema.Tuple([Schema.optionalKey(Schema.String)])".to_string(),
            "readonly [(string)?]",
        ),
        // :647 — minItems marks the leading position required.
        (
            serde_json::json!({
                "type": "array",
                "prefixItems": [{"type": "string"}],
                "minItems": 1,
                "maxItems": 1
            }),
            "Schema.Tuple([Schema.String])".to_string(),
            "readonly [string]",
        ),
        // :661 — `items` names the open element.
        (
            serde_json::json!({
                "type": "array",
                "prefixItems": [{"type": "string"}],
                "minItems": 1,
                "items": {"type": "number"}
            }),
            r#"Schema.TupleWithRest(Schema.Tuple([Schema.String]), [Schema.Number.check(Schema.isFinite().annotate({ "expected": "a finite number" }))])"#.to_string(),
            "readonly [string, ...Array<number>]",
        ),
    ];

    for (doc, code, ty) in cases {
        let schema = Schema::<Unknown>::from_json_schema(&doc).expect("imports");
        assert_eq!(schema.to_code(), code, "code for {doc:?}");
        assert_eq!(schema.to_type(), ty, "type for {doc:?}");
    }

    // A closed tuple holds its arity, and the rest form absorbs past the
    // prefix: the two are different shapes, not the same shape written twice.
    let closed = Schema::<Unknown>::from_json_schema(&serde_json::json!({
        "type": "array",
        "prefixItems": [{"type": "string"}],
        "maxItems": 1
    }))
    .expect("imports a closed tuple");
    assert!(
        closed
            .decode(Unknown::Array(vec![Unknown::Text("a".into())]))
            .is_ok()
    );
    assert!(closed.decode(Unknown::Array(Vec::new())).is_ok());
    assert!(
        closed
            .decode(Unknown::Array(vec![
                Unknown::Text("a".into()),
                Unknown::Text("b".into()),
            ]))
            .is_err()
    );

    let open = Schema::<Unknown>::from_json_schema(&serde_json::json!({
        "type": "array",
        "prefixItems": [{"type": "string"}],
        "items": {"type": "number"}
    }))
    .expect("imports an open tuple");
    assert!(
        open.decode(Unknown::Array(vec![Unknown::Text("a".into())]))
            .is_ok()
    );
    assert!(
        open.decode(Unknown::Array(vec![
            Unknown::Text("a".into()),
            Unknown::Number(1.0),
        ]))
        .is_ok()
    );
}

#[test]
fn json_schema_document_intersection_rows_carry_their_code_and_type() {
    // fromJsonSchemaDocument.test.ts's allOf intersection families. A member's
    // annotation rides the checks it contributed: the one check it left behind,
    // the base when it left none (its only check was a duplicate), or a filter
    // group when it left several. Each case below is a row from the file.
    use elm_effect_schema::{Schema, Unknown};

    let cases: Vec<(serde_json::Value, &str)> = vec![
        // :1892 — a member contributing one check and no annotation.
        (
            serde_json::json!({"type": "string", "allOf": [{"minLength": 1}]}),
            r#"Schema.String.check(Schema.isMinLength(1).annotate({ "expected": "a value with a length of at least 1" }))"#,
        ),
        // :1911 — the member's description rides its own check.
        (
            serde_json::json!({"type": "string", "allOf": [{"minLength": 1, "description": "b"}]}),
            r#"Schema.String.check(Schema.isMinLength(1).annotate({ "expected": "a value with a length of at least 1", "description": "b" }))"#,
        ),
        // :1985 — the base already has a check, so the member's is appended.
        (
            serde_json::json!({"type": "string", "maxLength": 2, "allOf": [{"minLength": 1}]}),
            r#"Schema.String.check(Schema.isMaxCodePoints(2).annotate({ "expected": "a string with at most 2 code points" })).check(Schema.isMinLength(1).annotate({ "expected": "a value with a length of at least 1" }))"#,
        ),
        // :2005 — the base's own annotation leads, before any check.
        (
            serde_json::json!({
                "type": "string",
                "description": "a",
                "maxLength": 2,
                "allOf": [{"minLength": 1}]
            }),
            r#"Schema.String.annotate({ "description": "a" }).check(Schema.isMaxCodePoints(2).annotate({ "expected": "a string with at most 2 code points" })).check(Schema.isMinLength(1).annotate({ "expected": "a value with a length of at least 1" }))"#,
        ),
        // :2179 — the member's only check duplicates the base's, so its
        // annotation moves to the base instead.
        (
            serde_json::json!({"type": "number", "allOf": [{"type": "number", "description": "b"}]}),
            r#"Schema.Number.annotate({ "description": "b" }).check(Schema.isFinite().annotate({ "expected": "a finite number" }))"#,
        ),
        // :2196 — an integer member stacks `isInt` on a number base.
        (
            serde_json::json!({"type": "number", "allOf": [{"type": "integer"}]}),
            r#"Schema.Number.check(Schema.isFinite().annotate({ "expected": "a finite number" })).check(Schema.isInt().annotate({ "expected": "an integer" }))"#,
        ),
        // :2215 — two members, each keeping its own checks, with the duplicate
        // `isInt` collapsed.
        (
            serde_json::json!({
                "type": "number",
                "allOf": [
                    {"type": "integer", "minimum": 2},
                    {"type": "integer", "maximum": 2}
                ]
            }),
            r#"Schema.Number.check(Schema.isFinite().annotate({ "expected": "a finite number" })).check(Schema.isInt().annotate({ "expected": "an integer" })).check(Schema.isGreaterThanOrEqualTo(2).annotate({ "expected": "a value greater than or equal to 2" })).check(Schema.isLessThanOrEqualTo(2).annotate({ "expected": "a value less than or equal to 2" }))"#,
        ),
        // :1930 — the base's own annotation leads, and the member contributes no
        // annotation of its own.
        (
            serde_json::json!({"type": "string", "description": "a", "allOf": [{"minLength": 1}]}),
            r#"Schema.String.annotate({ "description": "a" }).check(Schema.isMinLength(1).annotate({ "expected": "a value with a length of at least 1" }))"#,
        ),
        // :1950 — a member with only a description: no check survives, so the
        // annotation lands on the base.
        (
            serde_json::json!({"type": "string", "description": "a", "allOf": [{"description": "b"}]}),
            r#"Schema.String.annotate({ "description": "b" })"#,
        ),
        // :2026 — a base annotation and a member annotation on the member's check.
        (
            serde_json::json!({
                "type": "string",
                "description": "a",
                "maxLength": 2,
                "allOf": [{"minLength": 1, "description": "b"}]
            }),
            r#"Schema.String.annotate({ "description": "a" }).check(Schema.isMaxCodePoints(2).annotate({ "expected": "a string with at most 2 code points" })).check(Schema.isMinLength(1).annotate({ "expected": "a value with a length of at least 1", "description": "b" }))"#,
        ),
        // :2047 — a member contributing two checks and no annotation stays a
        // linear chain.
        (
            serde_json::json!({"type": "string", "allOf": [{"minLength": 1, "maxLength": 2}]}),
            r#"Schema.String.check(Schema.isMinLength(1).annotate({ "expected": "a value with a length of at least 1" })).check(Schema.isMaxCodePoints(2).annotate({ "expected": "a string with at most 2 code points" }))"#,
        ),
        // :2066 — the same two checks WITH an annotation collapse into a filter
        // group, which is the only place the pin uses one.
        (
            serde_json::json!({"type": "string", "allOf": [{"minLength": 1, "maxLength": 2, "description": "b"}]}),
            r#"Schema.String.check(Schema.makeFilterGroup([Schema.isMinLength(1).annotate({ "expected": "a value with a length of at least 1" }), Schema.isMaxCodePoints(2).annotate({ "expected": "a string with at most 2 code points" })]).annotate({ "description": "b" }))"#,
        ),
        // :2085 — a nested allOf, where the inner description rides the inner
        // check rather than the outer member.
        (
            serde_json::json!({
                "type": "string",
                "allOf": [{"minLength": 1, "allOf": [{"maxLength": 2, "description": "c"}]}]
            }),
            r#"Schema.String.check(Schema.isMinLength(1).annotate({ "expected": "a value with a length of at least 1" })).check(Schema.isMaxCodePoints(2).annotate({ "expected": "a string with at most 2 code points", "description": "c" }))"#,
        ),
        // :2235 — an integer base with a number member, so the order inverts.
        (
            serde_json::json!({"type": "integer", "allOf": [{"type": "number"}]}),
            r#"Schema.Number.check(Schema.isInt().annotate({ "expected": "an integer" })).check(Schema.isFinite().annotate({ "expected": "a finite number" }))"#,
        ),
    ];

    for (doc, code) in cases {
        let schema = Schema::<Unknown>::from_json_schema(&doc).expect("imports");
        assert_eq!(schema.to_code(), code, "code for {doc:?}");
    }

    // The checks survive the intersection as enforcement, not just as text: a
    // value failing the member's check is refused by the intersection.
    let schema = Schema::<Unknown>::from_json_schema(&serde_json::json!({
        "type": "string",
        "minLength": 2,
        "allOf": [{"minLength": 3}]
    }))
    .expect("imports");
    assert!(schema.decode(Unknown::Text("abc".to_string())).is_ok());
    assert!(schema.decode(Unknown::Text("ab".to_string())).is_err());
}

#[test]
fn json_schema_document_object_checks_carry_their_code_and_type() {
    // fromJsonSchemaDocument.test.ts's object `checks` and `patternProperties`
    // families. A bound on the entry count and a schema every key must satisfy
    // both ride a record, which is the shape an open object already has; a
    // pattern-keyed mapping is a record whose KEY is checked rather than passed
    // through.
    use elm_effect_schema::{Schema, Unknown};

    let cases: Vec<(serde_json::Value, &str)> = vec![
        // :1026 — a lower bound on the entry count.
        (
            serde_json::json!({"type": "object", "minProperties": 1}),
            r#"Schema.Record(Schema.String, Schema.Json.annotate({ "expected": "JSON value" })).check(Schema.isMinProperties(1).annotate({ "expected": "a value with at least 1 entry" }))"#,
        ),
        // :1038 — an upper bound.
        (
            serde_json::json!({"type": "object", "maxProperties": 1}),
            r#"Schema.Record(Schema.String, Schema.Json.annotate({ "expected": "JSON value" })).check(Schema.isMaxProperties(1).annotate({ "expected": "a value with at most 1 entry" }))"#,
        ),
        // :1052 — `propertyNames`: the key schema stands in the check's argument.
        (
            serde_json::json!({"type": "object", "propertyNames": {"pattern": "^[A-Z]"}}),
            r#"Schema.Record(Schema.String, Schema.Json.annotate({ "expected": "JSON value" })).check(Schema.isPropertyNames(Schema.String.check(Schema.isPattern(new RegExp("^[A-Z]", "u")).annotate({ "expected": "a string matching the RegExp ^[A-Z]" }))).annotate({ "expected": "an object with property names matching the schema" }))"#,
        ),
        // :973 — a pattern-keyed mapping, where the value schema is `never`.
        (
            serde_json::json!({
                "type": "object",
                "patternProperties": {"^a": {"not": {}}},
                "additionalProperties": false
            }),
            r#"Schema.Record(Schema.String.check(Schema.isPattern(new RegExp("^a", "u")).annotate({ "expected": "a string matching the RegExp ^a" })), Schema.Never)"#,
        ),
        // :988 — the same pattern with a string value schema.
        (
            serde_json::json!({
                "type": "object",
                "patternProperties": {"^a": {"type": "string"}},
                "additionalProperties": false
            }),
            r#"Schema.Record(Schema.String.check(Schema.isPattern(new RegExp("^a", "u")).annotate({ "expected": "a string matching the RegExp ^a" })), Schema.String)"#,
        ),
        // :380 — the `pattern` keyword renders the same RegExp literal.
        (
            serde_json::json!({"type": "string", "pattern": "a*"}),
            r#"Schema.String.check(Schema.isPattern(new RegExp("a*", "u")).annotate({ "expected": "a string matching the RegExp a*" }))"#,
        ),
    ];

    for (doc, code) in cases {
        let schema = Schema::<Unknown>::from_json_schema(&doc).expect("imports");
        assert_eq!(schema.to_code(), code, "code for {doc:?}");
    }

    // Two patterns cannot become one key schema, so the pin refuses rather than
    // silently keeping one.
    assert!(
        Schema::<Unknown>::from_json_schema(&serde_json::json!({
            "type": "object",
            "patternProperties": {"a*": {"type": "string"}, "b*": {"type": "number"}}
        }))
        .is_err()
    );
}

#[test]
fn json_schema_document_property_names_rows_carry_their_code_and_type() {
    // fromJsonSchemaDocument.test.ts's `propertyNames` family. `false` as the key
    // schema is `Never`, and two members each carrying a `propertyNames` check
    // stack their checks rather than one replacing the other.
    use elm_effect_schema::{Schema, Unknown};

    let cases: Vec<(serde_json::Value, &str)> = vec![
        // :1069 — a key schema admitting no key.
        (
            serde_json::json!({"type": "object", "propertyNames": false}),
            r#"Schema.Record(Schema.String, Schema.Json.annotate({ "expected": "JSON value" })).check(Schema.isPropertyNames(Schema.Never).annotate({ "expected": "an object with property names matching the schema" }))"#,
        ),
        // :1086 — two members, each contributing its own `isPropertyNames`.
        (
            serde_json::json!({
                "allOf": [
                    {"type": "object", "propertyNames": {"pattern": "^[A-Z]"}},
                    {"type": "object", "propertyNames": {"minLength": 2}}
                ]
            }),
            r#"Schema.Record(Schema.String, Schema.Json.annotate({ "expected": "JSON value" })).check(Schema.isPropertyNames(Schema.String.check(Schema.isPattern(new RegExp("^[A-Z]", "u")).annotate({ "expected": "a string matching the RegExp ^[A-Z]" }))).annotate({ "expected": "an object with property names matching the schema" })).check(Schema.isPropertyNames(Schema.String.check(Schema.isMinCodePoints(2).annotate({ "expected": "a string with at least 2 code points" }))).annotate({ "expected": "an object with property names matching the schema" }))"#,
        ),
    ];

    for (doc, code) in cases {
        let schema = Schema::<Unknown>::from_json_schema(&doc).expect("imports");
        assert_eq!(schema.to_code(), code, "code for {doc:?}");
    }

    // A boolean schema is JSON Schema's shorthand, so it is decided at import
    // rather than by a special case at each use.
    let never = Schema::<Unknown>::from_json_schema(&serde_json::json!(false)).expect("imports");
    assert_eq!(never.to_code(), "Schema.Never");
    let any = Schema::<Unknown>::from_json_schema(&serde_json::json!(true)).expect("imports");
    assert_eq!(
        any.to_code(),
        r#"Schema.Json.annotate({ "expected": "JSON value" })"#
    );
}

#[test]
fn json_schema_document_string_checks_count_code_points_and_integers_stay_safe() {
    // fromJsonSchemaDocument.test.ts:82 — the row's table drives `Schema.is`
    // over the imported schema. Its law is the counting: `minLength` other
    // than 1 and `maxLength` count code points, so a two-code-unit emoji is
    // one code point and a base plus combining mark is two, while `integer`
    // is a safe integer rather than any integral double.
    use elm_effect_schema::{Schema, Unknown};

    fn accepts(doc: &serde_json::Value, input: Unknown) -> bool {
        Schema::<Unknown>::from_json_schema(doc)
            .expect("imports")
            .decode(input)
            .is_ok()
    }

    let smiley = Unknown::Text("\u{1F600}".to_string());
    let combining = Unknown::Text("e\u{0301}".to_string());
    // Rust cannot hold an unpaired surrogate, so the pin's lone-surrogate
    // inputs arrive as the lossy replacement character: one code point, which
    // is the count the row's two expectations turn on.
    let lone_surrogate = Unknown::Text(String::from_utf16_lossy(&[0xD800]));
    let accent = Unknown::Text("\u{00E9}".to_string());
    let empty = Unknown::Text(String::new());
    let one = Unknown::Text("a".to_string());
    for (doc, input, expected) in [
        (
            serde_json::json!({"type": "string", "minLength": 2}),
            smiley.clone(),
            false,
        ),
        (
            serde_json::json!({"type": "string", "maxLength": 1}),
            smiley.clone(),
            true,
        ),
        (
            serde_json::json!({"type": "string", "minLength": 1}),
            smiley.clone(),
            true,
        ),
        (
            serde_json::json!({"type": "string", "minLength": 1}),
            empty.clone(),
            false,
        ),
        (
            serde_json::json!({"type": "string", "maxLength": 0}),
            empty.clone(),
            true,
        ),
        (
            serde_json::json!({"type": "string", "maxLength": 0}),
            one.clone(),
            false,
        ),
        (
            serde_json::json!({"type": "string", "minLength": 2}),
            accent.clone(),
            false,
        ),
        (
            serde_json::json!({"type": "string", "maxLength": 1}),
            accent.clone(),
            true,
        ),
        (
            serde_json::json!({"type": "string", "minLength": 2}),
            combining.clone(),
            true,
        ),
        (
            serde_json::json!({"type": "string", "maxLength": 1}),
            combining.clone(),
            false,
        ),
        (
            serde_json::json!({"type": "string", "minLength": 2}),
            lone_surrogate.clone(),
            false,
        ),
        (
            serde_json::json!({"type": "string", "maxLength": 1}),
            lone_surrogate.clone(),
            true,
        ),
        (
            serde_json::json!({"type": "integer"}),
            Unknown::Number(1e20),
            false,
        ),
        (
            serde_json::json!({"type": "integer"}),
            Unknown::Number(1.0),
            true,
        ),
        (
            serde_json::json!({"type": "string", "pattern": "^.$"}),
            smiley.clone(),
            true,
        ),
    ] {
        assert_eq!(
            accepts(&doc, input),
            expected,
            "the row's table entry did not match for {doc:?}"
        );
    }
}

#[test]
fn json_schema_document_string_refinements_filter_enum_members_and_patterns_match_a_code_point() {
    // fromJsonSchemaDocument.test.ts:106 — an imported string refinement is
    // applied to the exact values an `enum` or `const` names, in code points:
    // the two-code-unit emoji drops out of a two-code-point enum and the
    // one-code-point pattern accepts it.
    use elm_effect_schema::{Schema, Unknown};

    let min_length = Schema::<Unknown>::from_json_schema(&serde_json::json!({
        "enum": ["\u{1F600}", "e\u{0301}"],
        "minLength": 2
    }))
    .expect("imports");
    assert!(
        min_length
            .decode(Unknown::Text("e\u{0301}".to_string()))
            .is_ok()
    );
    assert!(
        min_length
            .decode(Unknown::Text("\u{1F600}".to_string()))
            .is_err()
    );

    let pattern = Schema::<Unknown>::from_json_schema(&serde_json::json!({
        "const": "\u{1F600}",
        "pattern": "^.$"
    }))
    .expect("imports");
    assert!(
        pattern
            .decode(Unknown::Text("\u{1F600}".to_string()))
            .is_ok()
    );
}

#[test]
fn json_schema_document_refuses_a_pattern_outside_ecmascript_unicode_mode() {
    // fromJsonSchemaDocument.test.ts:119 — a pattern ECMAScript Unicode mode
    // rejects is a typed import failure naming the keyword's path, not an
    // unenforced check the caller only discovers later.
    use elm_effect_schema::{Schema, Unknown};

    // `Schema` carries no `Debug`, so the refusal is matched rather than
    // `expect_err`ed.
    let Err(error) = Schema::<Unknown>::from_json_schema(&serde_json::json!({
        "type": "string",
        "pattern": "\\a"
    })) else {
        panic!("a pattern outside ECMAScript Unicode mode must refuse the import")
    };
    let printed = format!("{error}");
    assert!(
        printed.contains("Cannot import pattern using ECMAScript Unicode mode"),
        "{printed}"
    );
    assert!(printed.contains("pattern"), "{printed}");
}

#[test]
fn json_schema_document_min_length_imports_as_code_points_unless_it_is_one() {
    // fromJsonSchemaDocument.test.ts:82 — the pin's rule for `minLength` is
    // that 1 stays `isMinLength` (the non-empty case) and every other value
    // becomes `isMinCodePoints`, so the code names which was chosen.
    use elm_effect_schema::{Schema, Unknown};

    for (doc, code) in [
        (
            serde_json::json!({"type": "string", "minLength": 1}),
            r#"Schema.String.check(Schema.isMinLength(1).annotate({ "expected": "a value with a length of at least 1" }))"#,
        ),
        (
            serde_json::json!({"type": "string", "minLength": 2}),
            r#"Schema.String.check(Schema.isMinCodePoints(2).annotate({ "expected": "a string with at least 2 code points" }))"#,
        ),
        (
            serde_json::json!({"type": "string", "minLength": 0}),
            r#"Schema.String.check(Schema.isMinCodePoints(0).annotate({ "expected": "a string with at least 0 code points" }))"#,
        ),
    ] {
        let schema = Schema::<Unknown>::from_json_schema(&doc).expect("imports");
        assert_eq!(schema.to_code(), code, "code for {doc:?}");
    }
}

#[test]
fn json_schema_document_refuses_structured_const_and_enum_members() {
    // fromJsonSchemaDocument.test.ts:220 and :1134 — a `const` or an `enum`
    // member that is an object or an array has no literal form, so the import
    // refuses it naming the keyword and the member's index. An `enum` beside a
    // `const` is still checked, and its own index is the one reported.
    use elm_effect_schema::{Schema, Unknown};

    for doc in [
        serde_json::json!({"const": {}}),
        serde_json::json!({"const": []}),
    ] {
        let Err(error) = Schema::<Unknown>::from_json_schema(&doc) else {
            panic!("a structured const must refuse the import: {doc:?}")
        };
        let printed = format!("{error}");
        assert!(
            printed.contains("only primitive values are supported in \"const\" and \"enum\""),
            "{printed}"
        );
        assert!(printed.contains("$.const"), "{printed}");
    }
    for doc in [
        serde_json::json!({"enum": ["a", {}]}),
        serde_json::json!({"enum": ["a", []]}),
        serde_json::json!({"enum": ["a", {"not": "data"}]}),
    ] {
        let Err(error) = Schema::<Unknown>::from_json_schema(&doc) else {
            panic!("a structured enum member must refuse the import: {doc:?}")
        };
        let printed = format!("{error}");
        assert!(
            printed.contains("only primitive values are supported in \"const\" and \"enum\""),
            "{printed}"
        );
        assert!(printed.contains("$.enum[1]"), "{printed}");
    }
    // The `const` is a primitive, so it is the `enum` member that refuses.
    let Err(error) = Schema::<Unknown>::from_json_schema(&serde_json::json!({
        "const": "a",
        "enum": [{}]
    })) else {
        panic!("a structured enum member must refuse the import beside a const")
    };
    assert!(format!("{error}").contains("$.enum[0]"), "{error}");
}

#[test]
fn json_schema_document_preserves_never_in_items_prefix_items_and_union_members() {
    // fromJsonSchemaDocument.test.ts:58 — `not: {}` is `never`, and a `never`
    // stays a `never` wherever the document puts it: an array's element, a
    // prefix tuple's optional leading element, and a union member that itself
    // admits nothing.
    use elm_effect_schema::{Schema, Unknown};

    let items = Schema::<Unknown>::from_json_schema(&serde_json::json!({
        "type": "array",
        "items": {"not": {}}
    }))
    .expect("imports");
    assert_eq!(items.to_code(), "Schema.Array(Schema.Never)");
    assert_eq!(items.to_type(), "ReadonlyArray<never>");

    let prefix = Schema::<Unknown>::from_json_schema(&serde_json::json!({
        "type": "array",
        "prefixItems": [{"not": {}}]
    }))
    .expect("imports");
    assert_eq!(
        prefix.to_code(),
        r#"Schema.TupleWithRest(Schema.Tuple([Schema.optionalKey(Schema.Never)]), [Schema.Json.annotate({ "expected": "JSON value" })])"#
    );
    assert_eq!(
        prefix.to_type(),
        "readonly [(never)?, ...Array<Schema.Json>]"
    );

    let any_of = Schema::<Unknown>::from_json_schema(&serde_json::json!({
        "anyOf": [{"not": {}}, {"type": "string"}]
    }))
    .expect("imports");
    assert_eq!(
        any_of.to_code(),
        "Schema.Union([Schema.Never, Schema.String])"
    );
    assert_eq!(any_of.to_type(), "never | string");

    let one_of = Schema::<Unknown>::from_json_schema(&serde_json::json!({
        "oneOf": [{"not": {}}, {"type": "string"}]
    }))
    .expect("imports");
    assert_eq!(
        one_of.to_code(),
        r#"Schema.Union([Schema.Never, Schema.String], { mode: "oneOf" })"#
    );
}

#[test]
fn json_schema_document_keeps_annotation_only_documents_unconstrained() {
    // fromJsonSchemaDocument.test.ts:177 — a document carrying only
    // annotations constrains nothing, so the schema is the open JSON value
    // with those annotations riding it, `expected` first.
    use elm_effect_schema::{Schema, Unknown};

    let schema = Schema::<Unknown>::from_json_schema(&serde_json::json!({"format": "email"}))
        .expect("imports");
    assert_eq!(
        schema.to_code(),
        r#"Schema.Json.annotate({ "expected": "JSON value", "format": "email" })"#
    );
    assert_eq!(schema.to_type(), "Schema.Json");
}

#[test]
fn json_schema_document_integer_and_array_checks_carry_their_code_and_type() {
    // fromJsonSchemaDocument.test.ts's `type: integer` and `type: array`
    // `checks` groups: :488, :500, :512, :524, :536, :681, :693, :705, :717.
    // An integer stacks `isInt` under the same bound checks a plain number
    // takes, and an array's size and uniqueness checks are the length and
    // uniqueness filters the pin names. `uniqueItems: false` adds nothing,
    // which is the row that says so.
    use elm_effect_schema::{Schema, Unknown};

    let cases: Vec<(serde_json::Value, &str, &str)> = vec![
        (
            serde_json::json!({"type": "integer", "minimum": 1}),
            r#"Schema.Number.check(Schema.isInt().annotate({ "expected": "an integer" })).check(Schema.isGreaterThanOrEqualTo(1).annotate({ "expected": "a value greater than or equal to 1" }))"#,
            "number",
        ),
        (
            serde_json::json!({"type": "integer", "maximum": 1}),
            r#"Schema.Number.check(Schema.isInt().annotate({ "expected": "an integer" })).check(Schema.isLessThanOrEqualTo(1).annotate({ "expected": "a value less than or equal to 1" }))"#,
            "number",
        ),
        (
            serde_json::json!({"type": "integer", "exclusiveMinimum": 1}),
            r#"Schema.Number.check(Schema.isInt().annotate({ "expected": "an integer" })).check(Schema.isGreaterThan(1).annotate({ "expected": "a value greater than 1" }))"#,
            "number",
        ),
        (
            serde_json::json!({"type": "integer", "exclusiveMaximum": 1}),
            r#"Schema.Number.check(Schema.isInt().annotate({ "expected": "an integer" })).check(Schema.isLessThan(1).annotate({ "expected": "a value less than 1" }))"#,
            "number",
        ),
        (
            serde_json::json!({"type": "integer", "multipleOf": 1}),
            r#"Schema.Number.check(Schema.isInt().annotate({ "expected": "an integer" })).check(Schema.isMultipleOf(1).annotate({ "expected": "a value that is a multiple of 1" }))"#,
            "number",
        ),
        (
            serde_json::json!({"type": "array", "minItems": 1}),
            r#"Schema.Array(Schema.Json.annotate({ "expected": "JSON value" })).check(Schema.isMinLength(1).annotate({ "expected": "a value with a length of at least 1" }))"#,
            "ReadonlyArray<Schema.Json>",
        ),
        (
            serde_json::json!({"type": "array", "maxItems": 1}),
            r#"Schema.Array(Schema.Json.annotate({ "expected": "JSON value" })).check(Schema.isMaxLength(1).annotate({ "expected": "a value with a length of at most 1" }))"#,
            "ReadonlyArray<Schema.Json>",
        ),
        (
            serde_json::json!({"type": "array", "uniqueItems": true}),
            r#"Schema.Array(Schema.Json.annotate({ "expected": "JSON value" })).check(Schema.isUnique().annotate({ "expected": "an array with unique items" }))"#,
            "ReadonlyArray<Schema.Json>",
        ),
        (
            serde_json::json!({"type": "array", "uniqueItems": false}),
            r#"Schema.Array(Schema.Json.annotate({ "expected": "JSON value" }))"#,
            "ReadonlyArray<Schema.Json>",
        ),
    ];

    for (doc, code, ty) in cases {
        let schema = Schema::<Unknown>::from_json_schema(&doc).expect("imports");
        assert_eq!(schema.to_code(), code, "code for {doc:?}");
        assert_eq!(schema.to_type(), ty, "type for {doc:?}");
    }
}

#[test]
fn json_schema_document_array_type_only_items_and_true_all_of_members() {
    // fromJsonSchemaDocument.test.ts:560, :572, :1127 — a bare array constrains
    // its elements to JSON values, `items` names the element schema, and a
    // `true` member of an `allOf` constrains nothing so the intersection
    // leaves the other member standing.
    use elm_effect_schema::{Schema, Unknown};

    for (doc, code, ty) in [
        (
            serde_json::json!({"type": "array"}),
            r#"Schema.Array(Schema.Json.annotate({ "expected": "JSON value" }))"#,
            "ReadonlyArray<Schema.Json>",
        ),
        (
            serde_json::json!({"type": "array", "items": {"type": "string"}}),
            "Schema.Array(Schema.String)",
            "ReadonlyArray<string>",
        ),
        (
            serde_json::json!({"allOf": [true, {"type": "string"}]}),
            "Schema.String",
            "string",
        ),
    ] {
        let schema = Schema::<Unknown>::from_json_schema(&doc).expect("imports");
        assert_eq!(schema.to_code(), code, "code for {doc:?}");
        assert_eq!(schema.to_type(), ty, "type for {doc:?}");
    }
}

#[test]
fn json_schema_document_imports_an_array_of_type_names_as_a_union() {
    // fromJsonSchemaDocument.test.ts:1107 — an array of type names is a union
    // of the types it names, and a document annotation rides the union rather
    // than any one member.
    use elm_effect_schema::{Schema, Unknown};

    let plain = Schema::<Unknown>::from_json_schema(&serde_json::json!({
        "type": ["string", "null"]
    }))
    .expect("imports");
    assert_eq!(
        plain.to_code(),
        "Schema.Union([Schema.String, Schema.Null])"
    );
    assert_eq!(plain.to_type(), "string | null");
    // The union still decodes both of its members, which is the part a union
    // repr alone would not prove.
    assert!(plain.decode(Unknown::Text("a".to_string())).is_ok());
    assert!(plain.decode(Unknown::Null).is_ok());
    assert!(plain.decode(Unknown::Number(1.0)).is_err());

    let annotated = Schema::<Unknown>::from_json_schema(&serde_json::json!({
        "type": ["string", "null"],
        "description": "a"
    }))
    .expect("imports");
    assert_eq!(
        annotated.to_code(),
        r#"Schema.Union([Schema.String, Schema.Null]).annotate({ "description": "a" })"#
    );
}

#[test]
fn json_schema_document_refuses_the_two_object_forms_it_cannot_spell() {
    // fromJsonSchemaDocument.test.ts:863, :876, :888 — a typed
    // `additionalProperties` beside any other property schema is refused,
    // because an index signature also checks the keys the named fields or the
    // pattern already claimed, and a closed patterned object that also names
    // properties is refused for the same reason. Both name the reason.
    use elm_effect_schema::{Schema, Unknown};

    let typed = |extra: serde_json::Value| {
        let mut doc = serde_json::json!({
            "type": "object",
            "additionalProperties": {"type": "boolean"}
        });
        if let (Some(target), serde_json::Value::Object(fields)) = (doc.as_object_mut(), extra) {
            for (key, value) in fields {
                target.insert(key, value);
            }
        }
        doc
    };
    for doc in [
        typed(serde_json::json!({"properties": {"a": {"type": "string"}}, "required": ["a"]})),
        typed(serde_json::json!({"patternProperties": {"^a": {"type": "string"}}})),
    ] {
        let Err(error) = Schema::<Unknown>::from_json_schema(&doc) else {
            panic!("a typed additionalProperties beside other keys must refuse: {doc:?}")
        };
        let printed = format!("{error}");
        assert!(
            printed.contains(
                "Cannot combine typed \"additionalProperties\" with other property schemas"
            ),
            "{printed}"
        );
    }

    // The closed pattern form is supported only with one pattern and no
    // properties: a declared property or a required key both refuse it.
    for extra in [
        serde_json::json!({"properties": {"a": {"type": "number"}}}),
        serde_json::json!({"required": ["a"]}),
    ] {
        let mut doc = serde_json::json!({
            "type": "object",
            "patternProperties": {"^a": {"type": "number"}},
            "additionalProperties": false
        });
        if let (Some(target), serde_json::Value::Object(fields)) = (doc.as_object_mut(), extra) {
            for (key, value) in fields {
                target.insert(key, value);
            }
        }
        let Err(error) = Schema::<Unknown>::from_json_schema(&doc) else {
            panic!("a closed patterned object with properties must refuse: {doc:?}")
        };
        assert!(
            format!("{error}").contains("only one pattern without properties is supported"),
            "{error}"
        );
    }

    // The form the pin DOES support still imports: one pattern, closed, no
    // named properties. That is what makes the refusals above narrow rather
    // than a blanket ban on patterned objects. What the closed key domain
    // costs a decode is the excess-property family (:729, :903), which the
    // port does not carry yet.
    let supported = Schema::<Unknown>::from_json_schema(&serde_json::json!({
        "type": "object",
        "patternProperties": {"^a": {"type": "number"}},
        "additionalProperties": false
    }))
    .expect("imports");
    // The pattern becomes the key schema, which is the shape the port can
    // spell: one record whose key is a pattern check and whose value is the
    // one pattern schema.
    assert!(
        supported.to_code().contains("isPattern"),
        "{}",
        supported.to_code()
    );
    let inside = std::collections::BTreeMap::from([("abc".to_string(), Unknown::Number(1.0))]);
    assert!(supported.decode(Unknown::Object(inside)).is_ok());
}

#[test]
fn json_schema_document_a_grouped_range_refinement_decides_both_ends() {
    // fromJsonSchemaDocument.test.ts:1855 — a member carrying a minimum and a
    // maximum groups into one refinement, and the value it lands on is decided
    // by the group as a whole: inside the range decodes, outside it does not.
    use elm_effect_schema::{Schema, Unknown};

    let schema = Schema::<Unknown>::from_json_schema(&serde_json::json!({
        "type": "number",
        "allOf": [{"minimum": 1, "maximum": 2, "description": "range"}]
    }))
    .expect("imports");
    assert!(schema.decode(Unknown::Number(2.0)).is_ok());
    assert!(schema.decode(Unknown::Number(1.0)).is_ok());
    assert!(schema.decode(Unknown::Number(0.0)).is_err());
    assert!(schema.decode(Unknown::Number(3.0)).is_err());
}

#[test]
fn json_schema_document_filters_enum_members_in_either_order_and_keeps_a_typeless_member() {
    // fromJsonSchemaDocument.test.ts:1863, :1870, :1878, :2104 — a refinement
    // and an enum filter each other whichever side declares first, an `allOf`
    // with a single typeless member is just that member, and a nested `allOf`
    // inside a member groups its own checks the same way.
    use elm_effect_schema::{Schema, Unknown};

    for doc in [
        serde_json::json!({"type": "string", "minLength": 2, "allOf": [{"enum": ["a", "ab"]}]}),
        serde_json::json!({"enum": ["a", "ab"], "allOf": [{"type": "string", "minLength": 2}]}),
    ] {
        let schema = Schema::<Unknown>::from_json_schema(&doc).expect("imports");
        assert_eq!(
            schema.to_code(),
            r#"Schema.Literal("ab")"#,
            "code for {doc:?}"
        );
    }

    let typeless = Schema::<Unknown>::from_json_schema(&serde_json::json!({
        "allOf": [{"type": "string"}]
    }))
    .expect("imports");
    assert_eq!(typeless.to_code(), "Schema.String");

    let nested = Schema::<Unknown>::from_json_schema(&serde_json::json!({
        "type": "string",
        "allOf": [{"minLength": 1, "description": "b", "allOf": [{"maxLength": 2, "description": "c"}]}]
    }))
    .expect("imports");
    assert_eq!(
        nested.to_code(),
        r#"Schema.String.check(Schema.makeFilterGroup([Schema.isMinLength(1).annotate({ "expected": "a value with a length of at least 1" }), Schema.isMaxCodePoints(2).annotate({ "expected": "a string with at most 2 code points", "description": "c" })]).annotate({ "description": "b" }))"#
    );
}

#[test]
fn json_schema_document_groups_several_checks_behind_their_description() {
    // fromJsonSchemaDocument.test.ts:2254, :2273 — a member that leaves several
    // checks plus a description groups them into a `makeFilterGroup`, the
    // description rides the group, and an intersection that follows appends
    // its own check after the group rather than replacing it.
    use elm_effect_schema::{Schema, Unknown};

    let grouped = Schema::<Unknown>::from_json_schema(&serde_json::json!({
        "type": "number",
        "allOf": [
            {"minimum": 1, "description": "b", "allOf": [{"maximum": 2, "description": "c"}]}
        ]
    }))
    .expect("imports");
    assert_eq!(
        grouped.to_code(),
        r#"Schema.Number.check(Schema.isFinite().annotate({ "expected": "a finite number" })).check(Schema.makeFilterGroup([Schema.isGreaterThanOrEqualTo(1).annotate({ "expected": "a value greater than or equal to 1" }), Schema.isLessThanOrEqualTo(2).annotate({ "expected": "a value less than or equal to 2", "description": "c" })]).annotate({ "description": "b" }))"#
    );

    let continued = Schema::<Unknown>::from_json_schema(&serde_json::json!({
        "type": "number",
        "allOf": [
            {"minimum": 1, "maximum": 2, "description": "range"},
            {"type": "integer"}
        ]
    }))
    .expect("imports");
    assert_eq!(
        continued.to_code(),
        r#"Schema.Number.check(Schema.isFinite().annotate({ "expected": "a finite number" })).check(Schema.makeFilterGroup([Schema.isGreaterThanOrEqualTo(1).annotate({ "expected": "a value greater than or equal to 1" }), Schema.isLessThanOrEqualTo(2).annotate({ "expected": "a value less than or equal to 2" })]).annotate({ "description": "range" })).check(Schema.isInt().annotate({ "expected": "an integer" }))"#
    );
}

#[test]
fn json_schema_document_intersects_typed_bases_with_enums_and_literals() {
    // fromJsonSchemaDocument.test.ts:2123, :2149, :2163, :2335, :2347, :2359 —
    // a base that names a type meets an `enum` or a `const` by keeping the
    // values that fit the base: a string base keeps its string members and
    // drops a number one, a boolean base keeps booleans and refuses a number
    // literal outright, and two boolean bases are one boolean. The member's
    // own description rides the surviving literal.
    use elm_effect_schema::{Schema, Unknown};

    let cases: Vec<(serde_json::Value, &str)> = vec![
        (
            serde_json::json!({"type": "string", "allOf": [{"enum": ["a"]}]}),
            r#"Schema.Literal("a")"#,
        ),
        (
            serde_json::json!({
                "type": "string",
                "description": "a",
                "allOf": [{"enum": ["a"], "description": "b"}]
            }),
            r#"Schema.Literal("a").annotate({ "description": "b" })"#,
        ),
        (
            serde_json::json!({"type": "string", "allOf": [{"enum": ["a", "b"]}]}),
            r#"Schema.Literals(["a", "b"])"#,
        ),
        (
            serde_json::json!({"type": "string", "allOf": [{"enum": ["a", 1]}]}),
            r#"Schema.Literal("a")"#,
        ),
        (
            serde_json::json!({"type": "boolean", "allOf": [{"type": "boolean"}]}),
            "Schema.Boolean",
        ),
        (
            serde_json::json!({"type": "boolean", "allOf": [{"const": 1}]}),
            "Schema.Never",
        ),
        (
            serde_json::json!({"type": "boolean", "allOf": [{"enum": [true]}]}),
            "Schema.Literal(true)",
        ),
        (
            serde_json::json!({
                "type": "boolean",
                "description": "a",
                "allOf": [{"enum": [true], "description": "b"}]
            }),
            r#"Schema.Literal(true).annotate({ "description": "b" })"#,
        ),
        (
            serde_json::json!({"type": "number", "allOf": [{"enum": [1]}]}),
            "Schema.Literal(1)",
        ),
        (
            serde_json::json!({
                "type": "number",
                "description": "a",
                "allOf": [{"enum": [1], "description": "b"}]
            }),
            r#"Schema.Literal(1).annotate({ "description": "b" })"#,
        ),
        (
            serde_json::json!({"type": "number", "allOf": [{"enum": [1, 2]}]}),
            "Schema.Literals([1, 2])",
        ),
    ];

    for (doc, code) in cases {
        let schema = Schema::<Unknown>::from_json_schema(&doc).expect("imports");
        assert_eq!(schema.to_code(), code, "code for {doc:?}");
    }
}

#[test]
fn json_schema_document_literal_refinements_filter_through_both_orders() {
    // fromJsonSchemaDocument.test.ts:1827, :1831 — a refinement beside a
    // `const` filters that one value in either order, so the intersection is
    // the literal when it satisfies the refinement and `never` when it does
    // not. The two rows are the precision cases: a divisor below the
    // `toFixed` precision limit, and one with an operand far past it.
    use elm_effect_schema::{Schema, Unknown};

    for (doc, literal, kept) in [
        (
            serde_json::json!({"type": "number", "multipleOf": 1e-101}),
            0.0,
            true,
        ),
        (
            serde_json::json!({"type": "number", "multipleOf": 1e-101}),
            5e-102,
            false,
        ),
        (
            serde_json::json!({"type": "number", "multipleOf": 2}),
            1e21,
            true,
        ),
        (
            serde_json::json!({"type": "number", "multipleOf": 2}),
            1.0,
            false,
        ),
        // A subnormal divisor: the quotient is not exact, and plain float
        // arithmetic would call this value a multiple of it.
        (
            serde_json::json!({"type": "number", "multipleOf": 1e-323}),
            1.042e-321,
            false,
        ),
        (
            serde_json::json!({"type": "number", "multipleOf": 1e-323}),
            0.0,
            true,
        ),
    ] {
        for order in [
            [doc.clone(), serde_json::json!({"const": literal})],
            [serde_json::json!({"const": literal}), doc.clone()],
        ] {
            let schema = Schema::<Unknown>::from_json_schema(&serde_json::json!({"allOf": order}))
                .expect("imports");
            let code = schema.to_code();
            // The literal survives, or the intersection is empty. Which number
            // text a repr prints is the literal rendering's business, not this
            // row's.
            if kept {
                assert!(code.starts_with("Schema.Literal("), "{code} for {order:?}");
            } else {
                assert_eq!(code, "Schema.Never", "code for {order:?}");
            }
        }
    }
}

#[test]
fn json_schema_document_round_trips_an_optional_never_property() {
    // fromJsonSchemaDocument.test.ts:22 (#8137) — a struct field typed
    // `optionalKey(Never)` survives the trip out to a JSON Schema document and
    // back as an exact optional whose inner schema is still `never`, and the
    // object stays open the way the document said.
    use elm_effect_schema::{Field, Schema, Unknown};
    use std::collections::BTreeMap;

    let original = Schema::<BTreeMap<String, Unknown>>::struct_shape(vec![Field::optional_key(
        "value",
        Schema::as_unknown(Schema::never()),
    )]);
    let imported = Schema::<Unknown>::from_json_schema(&original.json_schema()).expect("imports");
    assert_eq!(
        imported.to_code(),
        r#"Schema.StructWithRest(Schema.Struct({ "value": Schema.optionalKey(Schema.Never) }), [Schema.Record(Schema.String, Schema.Json.annotate({ "expected": "JSON value" }))])"#
    );
    // The field is an exact optional, so an absent key decodes and a present
    // one still has to satisfy `never`.
    assert!(imported.decode(Unknown::Object(BTreeMap::new())).is_ok());
}

#[test]
fn json_schema_document_builds_unions_from_any_of_and_one_of() {
    // fromJsonSchemaDocument.test.ts:275, :304 — a bare `anyOf` is a union of
    // its members and a bare `oneOf` is the same union carrying its mode, so
    // the repr prints the options only for the mode that departs from the
    // default.
    use elm_effect_schema::{Schema, Unknown};

    let any_of = Schema::<Unknown>::from_json_schema(&serde_json::json!({
        "anyOf": [{"const": "a"}, {"enum": [1, 2]}]
    }))
    .expect("imports");
    assert_eq!(
        any_of.to_code(),
        r#"Schema.Union([Schema.Literal("a"), Schema.Literals([1, 2])])"#
    );
    assert_eq!(any_of.to_type(), "\"a\" | 1 | 2");

    let one_of = Schema::<Unknown>::from_json_schema(&serde_json::json!({
        "oneOf": [{"const": "a"}, {"enum": [1, 2]}]
    }))
    .expect("imports");
    assert_eq!(
        one_of.to_code(),
        r#"Schema.Union([Schema.Literal("a"), Schema.Literals([1, 2])], { mode: "oneOf" })"#
    );
}

#[test]
fn json_schema_document_refuses_a_union_beside_constraining_keywords() {
    // fromJsonSchemaDocument.test.ts:282, :316 — the keywords around an
    // `anyOf` or a `oneOf` intersect it, and the pin's rule distributes: each
    // union lane is pruned against the sibling, so the `id` property the
    // surrounding object requires lands in both members. The port carries no
    // lane pruning yet, so it refuses the document rather than dropping the
    // member's own constraints to produce a wrong schema.
    use elm_effect_schema::{Schema, Unknown};

    for keyword in ["anyOf", "oneOf"] {
        let doc = serde_json::json!({
            "type": "object",
            "properties": {"id": {"type": "string"}},
            "required": ["id"],
            keyword: [
                {"properties": {"a": {"type": "string"}}, "required": ["a"]},
                {"properties": {"b": {"type": "number"}}, "required": ["b"]}
            ]
        });
        let Err(error) = Schema::<Unknown>::from_json_schema(&doc) else {
            panic!("a union beside constraining keywords must refuse the import: {keyword}")
        };
        let printed = format!("{error}");
        assert!(
            printed.contains("lane pruning this port does not carry"),
            "{printed}"
        );
        assert!(printed.contains(&format!("$.{keyword}")), "{printed}");
    }
}

#[test]
fn json_schema_document_refuses_the_keywords_it_cannot_lower() {
    // fromJsonSchemaDocument.test.ts:1684, :3012, :3035, :3046, :3058 — the pin
    // refuses a keyword it cannot lower and names it, rather than importing a
    // schema that enforces less than the document asks for. The distinction is
    // not academic: an ignored `unevaluatedProperties: false` is
    // indistinguishable from an enforced one to whoever wrote the document.
    use elm_effect_schema::{Schema, Unknown};

    let refusal = |doc: &serde_json::Value, keyword: &str| {
        let Err(error) = Schema::<Unknown>::from_json_schema(doc) else {
            panic!("{doc} must be refused: {keyword}")
        };
        let printed = format!("{error}");
        assert!(
            printed.contains(&format!(
                "Cannot import JSON Schema keyword \"{keyword}\": Effect has no equivalent constraint."
            )),
            "{keyword}: {printed}"
        );
        assert!(
            printed.contains(&format!("$.{keyword}")),
            "{keyword}: {printed}"
        );
    };

    // :3035 — one ledger row over six keywords, so all six have to be here.
    for (keyword, value) in [
        ("$dynamicRef", serde_json::json!("#node")),
        ("contains", serde_json::json!({"type": "string"})),
        ("dependentRequired", serde_json::json!({"a": ["b"]})),
        (
            "dependentSchemas",
            serde_json::json!({"a": {"required": ["b"]}}),
        ),
        ("unevaluatedItems", serde_json::json!(false)),
        ("unevaluatedProperties", serde_json::json!(false)),
    ] {
        refusal(&serde_json::json!({keyword: value}), keyword);
    }

    // :3012 — `not` is lowered only when it is empty; the pin names the keyword.
    refusal(
        &serde_json::json!({
            "type": "object",
            "properties": {"value": {"not": {"type": "string"}}}
        }),
        "not",
    );

    // :3046 — both branches name `if`, which is the keyword that is live.
    for branch in ["then", "else"] {
        refusal(
            &serde_json::json!({"if": {"type": "string"}, branch: false}),
            "if",
        );
    }

    // :1684 — an unsupported keyword is refused inside a member too, and the
    // member is imported before the `false` sibling can make the intersection
    // `never`. The pin's own path is ["schema"]["allOf"][1]["contains"]; this
    // port reports the keyword it was importing when it stopped.
    refusal(
        &serde_json::json!({"allOf": [false, {"contains": {}}]}),
        "contains",
    );

    // :3058 — the same keywords are IGNORED when they apply to nothing, which is
    // the other half of the row: `false` for a conditional branch, and the two
    // `contains` cardinalities on their own. Refusing these would break
    // documents the pin accepts.
    for (keyword, value) in [
        ("if", serde_json::json!(false)),
        ("then", serde_json::json!(false)),
        ("else", serde_json::json!(false)),
        ("minContains", serde_json::json!(1)),
        ("maxContains", serde_json::json!(2)),
    ] {
        let schema = Schema::<Unknown>::from_json_schema(&serde_json::json!({keyword: value}))
            .unwrap_or_else(|error| panic!("{keyword} must be ignored: {error}"));
        assert_eq!(
            schema.to_code(),
            r#"Schema.Json.annotate({ "expected": "JSON value" })"#,
            "{keyword}"
        );
        assert_eq!(schema.to_type(), "Schema.Json", "{keyword}");
    }
}

#[test]
fn json_schema_document_answers_never_when_the_members_share_no_value() {
    // fromJsonSchemaDocument.test.ts:2570, :2577, :2584, :2591, :2598 — an
    // intersection with no shared value is `never`. The pin answers that rather
    // than refusing the document, and all five shapes hold nothing at all: a
    // `false` member, two different named types, and two different literals.
    use elm_effect_schema::{Schema, Unknown};

    for doc in [
        serde_json::json!({"allOf": [false, {"type": "string"}]}),
        serde_json::json!({"allOf": [{"type": "array"}, {"type": "string"}]}),
        serde_json::json!({"allOf": [{"type": "object"}, {"type": "string"}]}),
        serde_json::json!({"allOf": [{"type": "null"}, {"type": "string"}]}),
        serde_json::json!({"allOf": [{"const": 1}, {"const": 2}]}),
    ] {
        let schema = Schema::<Unknown>::from_json_schema(&doc).expect("imports");
        assert_eq!(schema.to_code(), "Schema.Never", "code for {doc}");
        assert_eq!(schema.to_type(), "never", "type for {doc}");
        // `never` holds no value, so the decoder refuses every shape the two
        // members named between them. This is the assertion the code string
        // cannot make on its own.
        for value in [
            Unknown::Null,
            Unknown::Bool(true),
            Unknown::Number(1.0),
            Unknown::Text("x".to_string()),
            Unknown::Array(Vec::new()),
            Unknown::Object(std::collections::BTreeMap::new()),
        ] {
            assert!(
                schema.decode(value.clone()).is_err(),
                "{doc} admitted {value:?}"
            );
        }
    }
}

#[test]
fn json_schema_document_intersects_two_sets_of_exact_values() {
    // Not a row on its own: the same code path as :2598, where the intersection
    // of two exact-value sets must be their intersection. This port kept
    // whichever side carried the values, so `{const:1}` beside `{enum:[1,2]}`
    // admitted 2, which neither member allows. The rows above catch the empty
    // case and this catches the non-empty one, which is the reason both are
    // here.
    use elm_effect_schema::{Schema, Unknown};

    let schema = Schema::<Unknown>::from_json_schema(&serde_json::json!({
        "allOf": [{"const": 1}, {"enum": [1, 2]}]
    }))
    .expect("imports");
    assert_eq!(schema.to_code(), "Schema.Literal(1)");
    assert_eq!(
        schema.decode(Unknown::Number(1.0)).expect("1"),
        Unknown::Number(1.0)
    );
    assert!(schema.decode(Unknown::Number(2.0)).is_err());
}

#[test]
fn json_schema_document_keeps_each_intersection_that_survives_its_member() {
    // fromJsonSchemaDocument.test.ts:1741, :1758, :2400, :2605, :2612, :2619,
    // :2626 — the `allOf` rows whose member does not restate the base in a
    // different shape. The member only adds an annotation, repeats a check the
    // base already carries, or names a value the base admits, so the base
    // survives with the member folded into it.
    //
    // The sibling rows are refused rather than carried: where the member
    // narrows the base's own shape (an array whose `prefixItems` differ, an
    // object with other `properties`), this port's `intersect` compares the two
    // `SchemaKind`s structurally and reports "a member with a different base"
    // where the pin merges the two shapes. Those rows are blocked on that fix,
    // not on their expectations, and asserting a refusal as the behavior would
    // pin the defect.
    use elm_effect_schema::{Schema, Unknown};

    let cases: Vec<(serde_json::Value, &str, &str)> = vec![
        (
            // :1741 — an untyped member carrying only a description annotates
            // the array rather than replacing it.
            serde_json::json!({"type": "array", "allOf": [{"description": "a"}]}),
            r#"Schema.Array(Schema.Json.annotate({ "expected": "JSON value" })).annotate({ "description": "a" })"#,
            "ReadonlyArray<Schema.Json>",
        ),
        (
            // :1758 — the same shape over a closed object.
            serde_json::json!({
                "type": "object",
                "additionalProperties": false,
                "allOf": [{"description": "a"}]
            }),
            r#"Schema.Record(Schema.String, Schema.Never).annotate({ "description": "a" })"#,
            "{ readonly [x: string]: never }",
        ),
        (
            // :2400 — a repeated `uniqueItems` stays one check, not two.
            serde_json::json!({
                "type": "array",
                "uniqueItems": true,
                "allOf": [{"uniqueItems": true}]
            }),
            r#"Schema.Array(Schema.Json.annotate({ "expected": "JSON value" })).check(Schema.isUnique().annotate({ "expected": "an array with unique items" }))"#,
            "ReadonlyArray<Schema.Json>",
        ),
        (
            // :2605 — null intersected with null is null.
            serde_json::json!({"allOf": [{"type": "null"}, {"type": "null"}]}),
            "Schema.Null",
            "null",
        ),
        (
            // :2612 — identical literals leave one of them.
            serde_json::json!({"allOf": [{"const": 1}, {"const": 1}]}),
            "Schema.Literal(1)",
            "1",
        ),
        (
            // :2619 — a literal the sibling's type admits survives.
            serde_json::json!({"allOf": [{"type": "boolean"}, {"const": true}]}),
            "Schema.Literal(true)",
            "true",
        ),
        (
            // :2626 — the same with the literal written first; the pin asserts
            // both orders, since neither side is privileged.
            serde_json::json!({"allOf": [{"const": true}, {"type": "boolean"}]}),
            "Schema.Literal(true)",
            "true",
        ),
    ];

    for (doc, code, ty) in &cases {
        let schema = Schema::<Unknown>::from_json_schema(doc).expect("imports");
        assert_eq!(&schema.to_code(), code, "code for {doc}");
        assert_eq!(&schema.to_type(), ty, "type for {doc}");
    }

    // The literal rows also decode: the surviving value is admitted and its
    // neighbour refused, so the code string cannot be the only thing holding the
    // row up.
    let one = Schema::<Unknown>::from_json_schema(&cases[4].0).expect("imports");
    assert_eq!(
        one.decode(Unknown::Number(1.0)).expect("1"),
        Unknown::Number(1.0)
    );
    assert!(one.decode(Unknown::Number(2.0)).is_err());

    for index in [5, 6] {
        let truthy = Schema::<Unknown>::from_json_schema(&cases[index].0).expect("imports");
        assert_eq!(
            truthy.decode(Unknown::Bool(true)).expect("true"),
            Unknown::Bool(true)
        );
        assert!(truthy.decode(Unknown::Bool(false)).is_err(), "{index}");
    }

    // The three rows whose member merges into the base (cases 0, 1 and 3) have
    // a DECODER that admits anything, so no decode can be asserted for them and
    // the code string is all there is to hold them. `intersect` rebuilds the
    // merged schema through `new_kinded`, and that function only builds a
    // checking decoder for `String`, `Number` and `Boolean`; every other kind
    // falls through to `|value| Ok(value)`. So
    // `{allOf:[{type:"null"},{type:"null"}]}` prints `Schema.Null` and decodes
    // a string, where a plain `{type:"null"}` refuses one. That is a defect in
    // the import path, not a limit of these rows: three of the four Never rows
    // are refused by the same function for the neighbouring reason, and the fix
    // belongs with them.
}

#[test]
fn json_schema_document_ref_rows_carry_their_code_and_type() {
    // fromJsonSchemaDocument.test.ts's `$ref` family. A reference is resolved
    // against the ROOT's `$defs`, refused when it names nothing, and its JSON
    // Pointer escapes are undone. Each case is one whole row.
    use elm_effect_schema::{Schema, Unknown};

    let refused = |doc: serde_json::Value| {
        Schema::<Unknown>::from_json_schema(&doc)
            .err()
            .map(|error| error.to_string())
            .unwrap_or_else(|| panic!("{doc:?} must refuse"))
    };

    // :1298 — an empty reference names nothing.
    let empty = refused(serde_json::json!({"$ref": ""}));
    assert!(empty.contains("empty reference"), "got: {empty}");

    // :1305 — an external reference is not a local definition.
    let external = refused(serde_json::json!({
        "$ref": "https://example.com/schema#/$defs/A",
        "$defs": {"A": {"type": "number"}}
    }));
    assert!(external.contains("another shape"), "got: {external}");

    // :1318 — a name the root does not define, naming which one was wanted.
    let missing = refused(serde_json::json!({"$ref": "#/$defs/Missing"}));
    assert!(missing.contains("Missing"), "got: {missing}");

    // :1325 — `~1` and `~0` are the JSON Pointer escapes for `/` and `~`, so a
    // definition may be named with either.
    let escaped = Schema::<Unknown>::from_json_schema(&serde_json::json!({
        "$ref": "#/$defs/A~1B~0C",
        "$defs": {"A/B~C": {"type": "string"}}
    }))
    .expect("imports");
    assert_eq!(escaped.to_code(), "Schema.String");

    // :1344 — the definition's own shape is what imports, and an `identifier`
    // annotation riding the reference rides the result.
    let named = Schema::<Unknown>::from_json_schema(&serde_json::json!({
        "$ref": "#/$defs/A_B",
        "$defs": {"A_B": {"type": "string", "identifier": "A B"}}
    }))
    .expect("imports");
    assert_eq!(
        named.to_code(),
        r#"Schema.String.annotate({ "identifier": "A B" })"#
    );

    // :1360 — an empty definition key is still a name, so `_` is not `""`.
    let underscore = Schema::<Unknown>::from_json_schema(&serde_json::json!({
        "$ref": "#/$defs/_",
        "$defs": {"_": {"type": "string", "identifier": ""}}
    }))
    .expect("imports");
    assert_eq!(
        underscore.to_code(),
        r#"Schema.String.annotate({ "identifier": "" })"#
    );

    // A definition is scoped to the root: a nested `$defs` is a different scope
    // and its name does not resolve.
    let scoped = refused(serde_json::json!({
        "$defs": {"Root": {"type": "string"}},
        "type": "object",
        "properties": {
            "child": {"$defs": {"Nested": {"type": "number"}}, "$ref": "#/$defs/Nested"}
        },
        "required": ["child"],
        "additionalProperties": false
    }));
    assert!(scoped.contains("Nested"), "got: {scoped}");
}

#[test]
fn json_schema_document_builtin_annotation_rows_carry_their_code_and_type() {
    // :1147 — the built-in JSON Schema annotations all ride the schema, in the
    // order the pin prints them. `contentSchema` is the one that is not a
    // string, so it is lifted as its raw JSON rather than as text.
    use elm_effect_schema::{Schema, Unknown};

    let schema = Schema::<Unknown>::from_json_schema(&serde_json::json!({
        "type": "string",
        "format": "email",
        "contentEncoding": "base64",
        "contentMediaType": "application/json",
        "contentSchema": {"type": "number"}
    }))
    .expect("imports");
    assert_eq!(
        schema.to_code(),
        r#"Schema.String.annotate({ "format": "email", "contentEncoding": "base64", "contentMediaType": "application/json", "contentSchema": { "type": "number" } })"#
    );
    assert_eq!(schema.to_type(), "string");
}
