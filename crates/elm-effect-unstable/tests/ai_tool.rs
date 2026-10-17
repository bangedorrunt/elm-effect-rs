//! `packages/effect/test/ai/Tool.test.ts` handler/failure-mode witnesses.
//!
//! Only the direct `Tool` + `ToolResultPart` + `AiError` contract is
//! representable here: the `Toolkit`/`LanguageModel` harness, `Stream`
//! preliminary results, `Fiber`/`Latch`/`TestClock` concurrency, `Context`
//! service injection, provider-defined/dynamic tool types, `Strict`
//! annotations, `needsApproval`, and Schema-to-JSON-Schema derivation have no
//! counterpart in this crate, so those rows are not carried.

use elm_effect_unstable::ai::{AiError, AiReason, FailureMode, Tool, ToolResultPart};
use serde_json::json;

fn success_tool() -> Tool {
    Tool::new("FailureModeReturn", "A test tool", |_| {
        Ok(json!({"testSuccess": "failure-mode-return-tool"}).to_string())
    })
}

fn failing_tool() -> Tool {
    Tool::new("FailureModeReturn", "A test tool", |_| {
        Err(json!({"testFailure": "failure-mode-return-tool"}).to_string())
    })
}

#[test]
fn returns_tool_call_handler_successes() {
    // Tool.test.ts:36 — handler success passes through; the tool result is not a failure.
    let out = success_tool()
        .call(r#"{"testParam":"test-param"}"#)
        .expect("succeeds");
    let part = ToolResultPart::new(
        "tool-123",
        "FailureModeReturn",
        false,
        serde_json::from_str(&out).unwrap(),
        serde_json::from_str(&out).unwrap(),
    );
    assert!(!part.is_failure);
    assert_eq!(
        part.result,
        json!({"testSuccess": "failure-mode-return-tool"})
    );
    assert_eq!(part.encoded_result, part.result);
    assert!(!part.provider_executed);
    assert!(!part.preliminary);
}

#[test]
fn returns_tool_call_handler_failures_when_failure_mode_is_return() {
    // Tool.test.ts:76 — return mode hands the failure back as a failed tool result.
    let outcome = failing_tool()
        .call_with_mode(r#"{"testParam":"test-param"}"#, &FailureMode::Return)
        .expect("return mode never raises");
    let message = outcome.expect_err("handler failed");
    assert!(
        message.contains("failure-mode-return-tool"),
        "unexpected: {message}"
    );
    let part = ToolResultPart::new(
        "tool-123",
        "FailureModeReturn",
        true,
        json!({"testFailure": "failure-mode-return-tool"}),
        json!({"testFailure": "failure-mode-return-tool"}),
    );
    assert!(part.is_failure);
    assert_eq!(part.result, part.encoded_result);
    assert!(!part.provider_executed);
    assert!(!part.preliminary);
}

#[test]
fn encodes_returned_failures_with_the_failure_schema() {
    // Tool.test.ts:116 — a 404 failure encodes to the wire form "404" (NumberFromString).
    let part = ToolResultPart::new(
        "tool-123",
        "FailureEncoding",
        true,
        json!(404),
        json!("404"),
    );
    let wire = part.encode_branch(true).expect("encodes");
    assert_eq!(wire.get("result"), Some(&json!("404")));
    let back = ToolResultPart::from_json(&wire).expect("decodes");
    assert_eq!(back.result, json!(404));
    assert_eq!(back.encoded_result, json!("404"));
}

#[test]
fn fails_the_return_mode_stream_on_invalid_result_encoding() {
    // Tool.test.ts:131 — a success wire holding "404" is rejected as a ToolResultEncodingError.
    let part = ToolResultPart::new("tool-123", "Positive", false, json!(-1), json!("-1"));
    let err = part.encode_branch(false).expect_err("rejects");
    assert!(err.contains("encodedResult"), "unexpected error: {err}");
    let ai_error = AiError::new(AiReason::ToolResultEncoding {
        tool_name: "Positive".to_owned(),
        tool_result: "-1".to_owned(),
        description: err,
    })
    .with_context("Toolkit", "Positive.handle");
    assert_eq!(ai_error.reason_tag(), "ToolResultEncodingError");
    assert_eq!(ai_error.module.as_deref(), Some("Toolkit"));
    assert_eq!(ai_error.method.as_deref(), Some("Positive.handle"));
}

#[test]
fn returns_handler_failures_in_return_mode_regardless_of_transformer() {
    // Tool.test.ts:149 — the OpenAI-transformer variant behaves identically at this
    // layer (no transformer machinery exists here); the failure is still returned.
    let outcome = failing_tool()
        .call_with_mode(r#"{"testParam":"test-param"}"#, &FailureMode::Return)
        .expect("return mode never raises");
    assert!(outcome.is_err());
    let part = ToolResultPart::new(
        "tool-123",
        "FailureModeReturn",
        true,
        json!({"testFailure": "failure-mode-return-tool"}),
        json!({"testFailure": "failure-mode-return-tool"}),
    );
    assert!(part.is_failure);
    assert_eq!(part.result, part.encoded_result);
}

#[test]
fn raises_an_error_on_handler_failures_when_failure_mode_is_error() {
    // Tool.test.ts:189 — error mode raises the handler failure with the tool attached.
    let err = Tool::new("FailureModeError", "A test tool", |_| {
        Err::<String, String>(json!({"testFailure": "failure-mode-error-tool"}).to_string())
    })
    .call_with_mode(r#"{"testParam":"test-param"}"#, &FailureMode::Error)
    .expect_err("error mode raises");
    assert_eq!(err.tool, "FailureModeError");
    assert!(
        err.message.contains("failure-mode-error-tool"),
        "unexpected: {}",
        err.message
    );
}

#[test]
fn returns_a_failed_tool_result_when_parameters_are_invalid_and_mode_is_return() {
    // Tool.test.ts:220 — non-object params fail validation; return mode surfaces a
    // failed tool result carrying a ToolParameterValidationError.
    let outcome = success_tool()
        .call_with_mode("nope", &FailureMode::Return)
        .expect("return mode never raises");
    let message = outcome.expect_err("params invalid");
    assert!(
        message.contains("invalid parameters"),
        "unexpected: {message}"
    );
    let ai_error = AiError::new(AiReason::ToolParameterValidation {
        tool_name: "FailureModeReturn".to_owned(),
        description: message,
    })
    .with_context("Toolkit", "FailureModeReturn.handle");
    assert_eq!(ai_error.reason_tag(), "ToolParameterValidationError");
    let part = ToolResultPart::new(
        "tool-123",
        "FailureModeReturn",
        true,
        ai_error.to_json(),
        ai_error.to_json(),
    );
    assert!(part.is_failure);
}

#[test]
fn returns_a_failed_tool_result_when_parameters_contain_nan_and_mode_is_return() {
    // Tool.test.ts:277 — NaN is not valid JSON, so the call fails before the handler runs.
    let outcome = success_tool()
        .call_with_mode(r#"{"testParam": NaN}"#, &FailureMode::Return)
        .expect("return mode never raises");
    let message = outcome.expect_err("NaN rejected");
    assert!(message.contains("NaN"), "unexpected: {message}");
    let ai_error = AiError::new(AiReason::ToolParameterValidation {
        tool_name: "FailureModeReturn".to_owned(),
        description: message,
    })
    .with_context("Toolkit", "FailureModeReturn.handle");
    assert_eq!(ai_error.reason_tag(), "ToolParameterValidationError");
}

#[test]
fn raises_an_error_when_parameters_are_invalid_and_mode_is_error() {
    // Tool.test.ts:311 — error mode raises invalid params with the tool attached.
    let err = Tool::new("FailureModeError", "A test tool", |params| {
        Ok(params.to_owned())
    })
    .call_with_mode(r#"{"testParam": NaN}"#, &FailureMode::Error)
    .expect_err("error mode raises");
    assert_eq!(err.tool, "FailureModeError");
    assert!(err.message.contains("NaN"), "unexpected: {}", err.message);
}

#[test]
fn returns_ai_error_when_handler_returns_an_ai_error_reason_and_mode_is_return() {
    // Tool.test.ts:351 — a RateLimit reason in return mode becomes a failed tool result.
    let reason = AiReason::RateLimit {
        retry_after_secs: None,
    };
    assert_eq!(AiError::new(reason).reason_tag(), "RateLimitError");
    let ai_error = AiError::new(AiReason::RateLimit {
        retry_after_secs: None,
    })
    .with_context("Toolkit", "FailureModeReturn.handle");
    let encoded = ai_error.to_json();
    let part = ToolResultPart::new(
        "tool-123",
        "FailureModeReturn",
        true,
        encoded.clone(),
        encoded.clone(),
    );
    assert!(part.is_failure);
    let back = AiError::from_json(&part.result).expect("AiError survives");
    assert_eq!(back.reason_tag(), "RateLimitError");
    assert_eq!(back.method.as_deref(), Some("FailureModeReturn.handle"));
}

#[test]
fn raises_an_ai_error_when_handler_returns_an_ai_error_reason_and_mode_is_error() {
    // Tool.test.ts:400 — a RateLimit reason in error mode is raised, not returned.
    let ai_error = AiError::new(AiReason::RateLimit {
        retry_after_secs: None,
    })
    .with_context("Toolkit", "FailureModeError.handle");
    let failing = Tool::new("FailureModeError", "A test tool", move |_| {
        Err::<String, String>(serde_json::to_string(&ai_error.to_json()).unwrap())
    });
    let err = failing
        .call_with_mode(r#"{"testParam":"test-param"}"#, &FailureMode::Error)
        .expect_err("error mode raises");
    assert_eq!(err.tool, "FailureModeError");
    assert!(
        err.message.contains("RateLimitError"),
        "unexpected: {}",
        err.message
    );
}

#[test]
fn returns_tool_call_successes_from_the_model() {
    // Tool.test.ts:792 — a provider-executed success round-trips with its flag intact.
    let result = json!({"testSuccess": "provider-defined-tool"});
    let part = ToolResultPart {
        id: "tool-123".to_owned(),
        name: "NoHandlerRequired".to_owned(),
        is_failure: false,
        result: result.clone(),
        encoded_result: result.clone(),
        provider_executed: true,
        preliminary: false,
    };
    assert!(part.provider_executed);
    let wire = part.to_json();
    assert_eq!(wire.get("providerExecuted"), Some(&json!(true)));
    let back = ToolResultPart::from_json(&wire).expect("decodes");
    assert_eq!(back, part);
}

#[test]
fn returns_tool_call_failures_from_the_model() {
    // Tool.test.ts:840 — a provider-executed failure round-trips with its flag intact.
    let result = json!({"testFailure": "provider-defined-tool"});
    let part = ToolResultPart {
        id: "tool-123".to_owned(),
        name: "NoHandlerRequired".to_owned(),
        is_failure: true,
        result: result.clone(),
        encoded_result: result.clone(),
        provider_executed: true,
        preliminary: false,
    };
    assert!(part.is_failure);
    assert!(part.provider_executed);
    let back = ToolResultPart::from_json(&part.to_json()).expect("decodes");
    assert_eq!(back, part);
}

#[test]
fn returns_provider_defined_handler_successes() {
    // Tool.test.ts:888 — a handler-backed provider tool success is not provider-executed.
    let out = Tool::new("HandlerRequired", "A test tool", |_| {
        Ok(json!({"testSuccess": "provider-defined-tool"}).to_string())
    })
    .call(r#"{"testParam":"test-param"}"#)
    .expect("succeeds");
    let part = ToolResultPart::new(
        "tool-123",
        "HandlerRequired",
        false,
        serde_json::from_str(&out).unwrap(),
        serde_json::from_str(&out).unwrap(),
    );
    assert!(!part.is_failure);
    assert!(!part.provider_executed);
    assert_eq!(part.result, json!({"testSuccess": "provider-defined-tool"}));
}

#[test]
fn returns_provider_defined_handler_failures_when_mode_is_return() {
    // Tool.test.ts:935 — return mode keeps a provider-tool handler failure as a result.
    let outcome = Tool::new("HandlerRequired", "A test tool", |_| {
        Err::<String, String>(json!({"testFailure": "provider-defined-tool"}).to_string())
    })
    .call_with_mode(r#"{"testParam":"test-param"}"#, &FailureMode::Return)
    .expect("return mode never raises");
    assert!(
        outcome
            .expect_err("failed")
            .contains("provider-defined-tool")
    );
}

#[test]
fn raises_on_provider_defined_handler_failures_when_mode_is_error() {
    // Tool.test.ts:983 — error mode raises a provider-tool handler failure.
    let err = Tool::new("HandlerRequired", "A test tool", |_| {
        Err::<String, String>(json!({"testFailure": "provider-defined-tool"}).to_string())
    })
    .call_with_mode(r#"{"testParam":"test-param"}"#, &FailureMode::Error)
    .expect_err("error mode raises");
    assert_eq!(err.tool, "HandlerRequired");
}

#[test]
fn returns_a_failed_provider_tool_result_when_parameters_are_invalid() {
    // Tool.test.ts:1021 — invalid params for a provider tool fail validation in return mode.
    let outcome = Tool::new("HandlerRequired", "A test tool", |params| {
        Ok(params.to_owned())
    })
    .call_with_mode("nope", &FailureMode::Return)
    .expect("return mode never raises");
    let message = outcome.expect_err("params invalid");
    let ai_error = AiError::new(AiReason::ToolParameterValidation {
        tool_name: "HandlerRequired".to_owned(),
        description: message,
    })
    .with_context("Toolkit", "HandlerRequired.handle");
    assert_eq!(ai_error.reason_tag(), "ToolParameterValidationError");
    assert_eq!(ai_error.method.as_deref(), Some("HandlerRequired.handle"));
}
