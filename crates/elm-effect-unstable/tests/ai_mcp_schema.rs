//! `packages/effect/test/ai/McpSchema.test.ts` metadata witnesses.

use elm_effect_unstable::mcp_schema::{CreateMessagePayload, ElicitFormRequest, RequestMeta};
use serde_json::json;

#[test]
fn preserves_custom_metadata_when_a_request_is_decoded_and_encoded() {
    // McpSchema.test.ts:6 — every progress-token shape roundtrips with custom values.
    for progress in [json!({}), json!({ "progressToken": "progress-1" }), json!({ "progressToken": 1 })] {
        let mut meta = progress.as_object().unwrap().clone();
        meta.insert("marker".to_owned(), json!("request"));
        meta.insert("custom".to_owned(), json!({ "values": [null, true, 42, "value"] }));
        let request = json!({ "_meta": meta });
        let decoded = RequestMeta::decode(&request).unwrap();
        assert_eq!(decoded.encode(), request);
    }
}

#[test]
fn allows_create_message_metadata_to_be_omitted() {
    // McpSchema.test.ts:19 — no metadata decodes without throwing.
    CreateMessagePayload::decode(&json!({ "messages": [], "maxTokens": 1 })).unwrap();
}

#[test]
fn requires_create_message_metadata_to_be_an_object() {
    // McpSchema.test.ts:23 — a string metadata is rejected.
    let err = CreateMessagePayload::decode(
        &json!({ "messages": [], "maxTokens": 1, "metadata": "invalid" }),
    )
    .unwrap_err();
    assert!(err.contains("metadata"), "unexpected error: {err}");
}

#[test]
fn preserves_every_november_elicitation_enum_form() {
    // McpSchema.test.ts:29 — untitled, titled, and legacy enum forms survive decode.
    let requested = json!({
        "mode": "form",
        "message": "Choose values",
        "requestedSchema": {
            "type": "object",
            "properties": {
                "untitled": { "type": "string", "enum": ["one", "two"] },
                "titled": { "type": "string", "oneOf": [{ "const": "one", "title": "One" }] },
                "legacy": { "type": "string", "enum": ["one"], "enumNames": ["One"] }
            }
        }
    });
    let decoded = ElicitFormRequest::decode(&requested).unwrap();
    assert_eq!(
        serde_json::to_value(&decoded.properties).unwrap(),
        requested["requestedSchema"]["properties"]
    );
}
