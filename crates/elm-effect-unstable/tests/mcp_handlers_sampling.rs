//! SamplingTest carry rows plus elicitation scaffolding (audio and
//! tool-enabled round-trip rows excused — need the full server harness).
use elm_effect_unstable::mcp_handlers::{ElicitFormRequest, ElicitUrlRequest, Sampler};

#[test]
fn echoes_canned_response_preserving_order() {
    let sampler = Sampler::new("test-model");
    let req = serde_json::json!({
        "messages": [
            { "role": "user", "content": { "type": "text", "text": "one" } },
            { "role": "user", "content": { "type": "text", "text": "two" } },
        ],
        "maxTokens": 10,
    });
    let res = sampler.create_message(&req).unwrap();
    assert_eq!(res["role"], serde_json::json!("assistant"));
    assert_eq!(res["model"], serde_json::json!("test-model"));
    assert_eq!(res["content"]["type"], serde_json::json!("text"));
    assert!(res["content"]["text"].as_str().unwrap().contains("one"));
}

#[test]
fn accepts_image_content_and_surfaces_client_errors() {
    let sampler = Sampler::new("m");
    let req = serde_json::json!({
        "messages": [{ "role": "user", "content": { "type": "image", "data": "aGk=", "mimeType": "image/png" } }],
        "maxTokens": 5,
    });
    assert!(sampler.create_message(&req).is_ok());
    assert!(sampler.create_message(&serde_json::json!({})).is_err());
}

#[test]
fn decodes_text_and_image_sampling_content() {
    assert_eq!(
        Sampler::decode_content(&serde_json::json!({ "type": "text", "text": "hi" })).unwrap(),
        "hi"
    );
    assert_eq!(
        Sampler::decode_content(
            &serde_json::json!({ "type": "image", "data": "aGk=", "mimeType": "image/png" })
        )
        .unwrap(),
        "aGk="
    );
    assert!(Sampler::decode_content(&serde_json::json!({ "type": "unknown" })).is_err());
}

#[test]
fn form_elicitation_validates_mode_message_and_schema() {
    let good = serde_json::json!({
        "mode": "form", "message": "fill in",
        "requestedSchema": { "type": "object", "properties": { "x": { "type": "string" } } },
    });
    let req = ElicitFormRequest::decode(&good).unwrap();
    assert_eq!(req.message, "fill in");
    assert_eq!(req.fields.len(), 1);
    assert!(
        ElicitFormRequest::decode(&serde_json::json!({ "mode": "url", "message": "x" })).is_err()
    );
    assert!(ElicitFormRequest::decode(&serde_json::json!({ "mode": "form" })).is_err());
}

#[test]
fn url_elicitation_requires_http_url_and_id() {
    let good = serde_json::json!({
        "mode": "url", "message": "open", "url": "https://example.com/e", "elicitationId": "e1",
    });
    let req = ElicitUrlRequest::decode(&good).unwrap();
    assert_eq!(req.url, "https://example.com/e");
    assert!(ElicitUrlRequest::decode(&serde_json::json!({ "mode": "url", "message": "x", "url": "ftp://e", "elicitationId": "e" })).is_err());
    assert!(
        ElicitUrlRequest::decode(
            &serde_json::json!({ "mode": "url", "message": "x", "url": "https://e" })
        )
        .is_err()
    );
}
