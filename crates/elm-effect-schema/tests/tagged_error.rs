//! TaggedError + catch_tag + JsonSchema fence.
//! Derives from Schema.ts (TaggedError shapes) and JsonSchema.ts rows.

use elm_effect_schema::{Schema, TaggedError, Unknown, catch_tag};

#[test]
fn tagged_error_decodes_its_tag() {
    let schema = Schema::tagged("NotFound");
    let mut fields = std::collections::BTreeMap::new();
    fields.insert("_tag".to_string(), Unknown::Text("NotFound".to_string()));
    fields.insert("id".to_string(), Unknown::Number(7.0));
    let error = schema.decode(Unknown::Object(fields)).unwrap();
    assert_eq!(error.tag, "NotFound");
    assert_eq!(error.fields["id"], Unknown::Number(7.0));
}

#[test]
fn wrong_tag_is_a_typed_error() {
    let schema = Schema::tagged("NotFound");
    let mut fields = std::collections::BTreeMap::new();
    fields.insert("_tag".to_string(), Unknown::Text("Gone".to_string()));
    let error = schema.decode(Unknown::Object(fields)).unwrap_err();
    assert!(error.to_string().contains("NotFound"));
}

#[test]
fn missing_tag_is_a_typed_error() {
    let schema = Schema::tagged("NotFound");
    let error = schema
        .decode(Unknown::Object(std::collections::BTreeMap::new()))
        .unwrap_err();
    assert!(error.to_string().contains("_tag"));
}

#[test]
fn catch_tag_recovers_matching_errors() {
    let recovered: Result<i32, TaggedError> =
        catch_tag(Err(TaggedError::new("Retryable")), "Retryable", |_| Ok(1));
    assert_eq!(recovered, Ok(1));
}

#[test]
fn catch_tag_ignores_other_tags() {
    let untouched: Result<i32, TaggedError> =
        catch_tag(Err(TaggedError::new("Fatal")), "Retryable", |_| Ok(1));
    assert!(untouched.is_err());
}

#[test]
fn catch_tag_leaves_success_alone() {
    let ok: Result<i32, TaggedError> = catch_tag(Ok(2), "Retryable", |_| Ok(1));
    assert_eq!(ok, Ok(2));
}

#[test]
fn json_schema_documents_primitives() {
    let doc = Schema::string().json_schema();
    assert_eq!(doc["type"], serde_json::json!("string"));
    let doc = Schema::number().json_schema();
    assert_eq!(doc["type"], serde_json::json!("number"));
}

#[test]
fn json_schema_documents_containers() {
    let doc = Schema::array(Schema::string()).json_schema();
    assert_eq!(doc["type"], serde_json::json!("array"));
    assert_eq!(doc["items"]["type"], serde_json::json!("string"));
    let doc = Schema::optional(Schema::number()).json_schema();
    assert!(doc["anyOf"].is_array());
}
