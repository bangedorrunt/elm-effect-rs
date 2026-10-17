//! `packages/effect/test/ai/Response.test.ts` response-part witnesses.

use elm_effect_unstable::ai::{
    AiError, AiReason, FinishPart, HttpRequestDetails, ResponseMetadataPart, ResponseUsage,
    ToolResultPart,
};
use serde_json::json;
use std::collections::HashMap;

#[test]
fn preserves_user_failure_fields_for_struct_unknown_and_record_schemas() {
    // Response.test.ts:22 — Toolkit keeps user failure fields; AllParts JSON keeps result/encodedResult.
    for failure in [
        json!({"type": "execution-denied", "reason": "r", "detail": "IMPORTANT"}),
        json!({"type": "execution-denied", "reason": "r", "detail": "IMPORTANT"}),
        json!({"type": "execution-denied", "reason": "r", "detail": "IMPORTANT"}),
    ] {
        let part = ToolResultPart::new(
            "tool-123",
            "UserFailure",
            true,
            failure.clone(),
            failure.clone(),
        );
        assert_eq!(part.result, failure);
        assert_eq!(part.encoded_result, failure);
        assert!(part.is_failure);
        assert!(!part.preliminary);
        let wire = part.to_json();
        let back = ToolResultPart::from_json(&wire).expect("decodes");
        assert_eq!(back.result, failure);
        assert_eq!(back.encoded_result, failure);
        assert_eq!(back, part);
    }
}

#[test]
fn round_trips_a_returned_ai_error_through_toolkit_and_all_parts_json() {
    // Response.test.ts:64 — AiError failure survives with its encoded form.
    let failure = AiError::new(AiReason::RateLimit {
        retry_after_secs: None,
    })
    .with_context("Test", "FrameworkFailure");
    let encoded_failure = failure.to_json();
    let part = ToolResultPart::new(
        "tool-123",
        "FrameworkFailure",
        true,
        serde_json::to_value(&encoded_failure).unwrap(),
        serde_json::to_value(&encoded_failure).unwrap(),
    );
    let wire = part.to_json();
    let json =
        serde_json::from_str::<serde_json::Value>(&serde_json::to_string(&wire).unwrap()).unwrap();
    let back = ToolResultPart::from_json(&json).expect("decodes");
    let decoded_error = AiError::from_json(&back.result).expect("AiError");
    assert_eq!(decoded_error.reason_tag(), "RateLimitError");
    assert_eq!(decoded_error.module.as_deref(), Some("Test"));
    assert_eq!(decoded_error.method.as_deref(), Some("FrameworkFailure"));
    assert_eq!(
        back.encoded_result,
        serde_json::to_value(&encoded_failure).unwrap()
    );
}

#[test]
fn round_trips_provider_defined_execution_failures_through_all_parts_json() {
    // Response.test.ts:128 — execution-denied / execution-interrupted survive AllParts JSON.
    for result in [
        json!({"type": "execution-denied", "reason": "User declined"}),
        json!({"type": "execution-interrupted", "reason": "Tool call execution was interrupted because the response finished with reason \"length\""}),
    ] {
        let part = ToolResultPart::new(
            "tool-123",
            "ProviderTool",
            true,
            result.clone(),
            result.clone(),
        );
        let wire = part.to_json();
        let json =
            serde_json::from_str::<serde_json::Value>(&serde_json::to_string(&wire).unwrap())
                .unwrap();
        let back = ToolResultPart::from_json(&json).expect("decodes");
        assert_eq!(back, part);
    }
}

#[test]
fn encodes_success_and_failure_branches_with_their_own_schema() {
    // Response.test.ts:170 — success encodes a number, failure encodes its string form.
    let success = ToolResultPart::new("tool-123", "ResultEncoding", false, json!(404), json!(404));
    assert_eq!(
        success.encode_branch(false).expect("encodes"),
        json!({
            "type": "tool-result", "id": "tool-123", "name": "ResultEncoding",
            "isFailure": false, "result": 404,
            "providerExecuted": false, "preliminary": false, "metadata": {},
        })
    );
    let failure = ToolResultPart::new("tool-123", "ResultEncoding", true, json!(404), json!("404"));
    assert_eq!(
        failure.encode_branch(true).expect("encodes"),
        json!({
            "type": "tool-result", "id": "tool-123", "name": "ResultEncoding",
            "isFailure": true, "result": "404",
            "providerExecuted": false, "preliminary": false, "metadata": {},
        })
    );
}

#[test]
fn rejects_an_encoded_result_from_the_other_branch() {
    // Response.test.ts:186 — a success wire rejects "404"; a failure wire rejects 404.
    let success = ToolResultPart::new(
        "tool-123",
        "ResultEncoding",
        false,
        json!(404),
        json!("404"),
    );
    let err = success.encode_branch(false).expect_err("rejects");
    assert!(err.contains("number"), "unexpected error: {err}");
    assert!(err.contains("encodedResult"), "unexpected error: {err}");
    let failure = ToolResultPart::new("tool-123", "ResultEncoding", true, json!(404), json!(404));
    let err = failure.encode_branch(true).expect_err("rejects");
    assert!(err.contains("string"), "unexpected error: {err}");
    assert!(err.contains("encodedResult"), "unexpected error: {err}");
}

#[test]
fn preserves_branch_encoded_results_through_an_all_parts_json_round_trip() {
    // Response.test.ts:192 — AllParts JSON keeps 404 and "404" on their branches.
    let success = ToolResultPart::new("tool-123", "ResultEncoding", false, json!(404), json!(404));
    let back = ToolResultPart::from_json(
        &serde_json::from_str(&serde_json::to_string(&success.to_json()).unwrap()).unwrap(),
    )
    .expect("decodes");
    assert_eq!(back, success);
    let failure = ToolResultPart::new("tool-123", "ResultEncoding", true, json!(404), json!("404"));
    let back = ToolResultPart::from_json(
        &serde_json::from_str(&serde_json::to_string(&failure.to_json()).unwrap()).unwrap(),
    )
    .expect("decodes");
    assert_eq!(back, failure);
}

#[test]
fn decodes_response_metadata_with_omitted_optional_fields() {
    // Response.test.ts:202 — `{type}` alone decodes to an empty metadata part.
    let decoded =
        ResponseMetadataPart::from_json(&json!({"type": "response-metadata"})).expect("decodes");
    assert_eq!(decoded, ResponseMetadataPart::new(None, None, None, None));
}

#[test]
fn round_trips_response_metadata_with_undefined_optional_fields_through_json() {
    // Response.test.ts:213 — undefined optionals stay omitted through JSON.
    let part = ResponseMetadataPart::new(None, None, None, None);
    let wire = part.to_json();
    let json =
        serde_json::from_str::<serde_json::Value>(&serde_json::to_string(&wire).unwrap()).unwrap();
    assert_eq!(json, json!({"metadata": {}, "type": "response-metadata"}));
    assert_eq!(
        ResponseMetadataPart::from_json(&json).expect("decodes"),
        part
    );
}

#[test]
fn round_trips_http_request_details_with_an_undefined_hash_through_json() {
    // Response.test.ts:233 — an undefined hash is omitted, not nulled.
    let request = HttpRequestDetails {
        method: "POST".to_owned(),
        url: "https://example.com/v1/responses".to_owned(),
        url_params: Vec::new(),
        hash: None,
        headers: HashMap::new(),
    };
    let wire = request.to_json();
    let json =
        serde_json::from_str::<serde_json::Value>(&serde_json::to_string(&wire).unwrap()).unwrap();
    assert_eq!(
        json,
        json!({
            "method": "POST", "url": "https://example.com/v1/responses",
            "urlParams": [], "headers": {},
        })
    );
    assert_eq!(
        HttpRequestDetails::from_json(&json).expect("decodes"),
        request
    );
}

#[test]
fn round_trips_a_finish_part_with_undefined_optional_fields_through_json() {
    // Response.test.ts:261 — undefined usage leaves and response stay omitted.
    let part = FinishPart::new(
        "stop",
        ResponseUsage::from_optional(
            &[
                ("uncached", None),
                ("total", None),
                ("cacheRead", None),
                ("cacheWrite", None),
            ],
            &[("total", None), ("text", None), ("reasoning", None)],
        ),
        None,
    );
    let wire = part.to_json();
    let json =
        serde_json::from_str::<serde_json::Value>(&serde_json::to_string(&wire).unwrap()).unwrap();
    assert_eq!(
        json,
        json!({
            "metadata": {}, "type": "finish", "reason": "stop",
            "usage": {"inputTokens": {}, "outputTokens": {}},
        })
    );
    assert_eq!(
        FinishPart::from_json(&json).expect("decodes"),
        FinishPart::new("stop", ResponseUsage::empty(), None)
    );
}
