//! Wave corpus: literal / union / boundary / optional-field shapes.
//! Ports packages/effect/test/schema/Schema.test.ts rows by exact behavior.

use elm_effect_schema::{Schema, Unknown};

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
fn union_tries_first_then_second() {
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
fn refine_non_empty_string() {
    let schema = Schema::string().refine("non-empty", |s: &String| !s.is_empty());
    assert!(schema.decode(Unknown::Text("x".to_string())).is_ok());
    assert!(schema.decode(Unknown::Text(String::new())).is_err());
}

#[test]
fn refine_is_base64() {
    let schema = Schema::string().refine("base64", |s: &String| {
        !s.is_empty()
            && s.len() % 4 == 0
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
