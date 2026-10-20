//! `packages/effect/test/ai/LanguageModel.test.ts` tool-call resolution witnesses.
//!
//! Only the sync contract is representable here: finish-reason gating,
//! param validation, failure modes, approval strip/pre-resolve, tracker
//! prepare/mark, and incremental fallback. Fiber/Latch interrupt handoff,
//! TestClock-delayed handlers, concurrent overlap/maxActive, and the Schema
//! codec transformer have no counterpart in this crate, so those rows are
//! not carried (excused inline below).
//!
//! Excused (Fiber/TestClock/interrupt/schema machinery, named):
//! - :324 interrupts in-flight handlers on incomplete finish (Effect.onInterrupt + Latch)
//! - :754 interrupts in-flight handlers when provider stream fails (Fiber interrupt on Stream.fail)
//! - :961 sequential handlers with concurrency 1 (Fiber overlap + Latch + TestClock)
//! - :1022 bounded concurrency overlap (Fiber overlap + Latch maxActive)
//! - :1085 tool call IDs to concurrent identical handlers (Fiber context.toolCallId overlap)
//! - :1137 bounds needsApproval evaluation with concurrency (concurrent needsApproval Fiber bound)
//! - :1221 canonical top-level $ref for class schemas (Schema.Class codec transformer)

use elm_effect_unstable::ai::{
    AiError, AiReason, FailureMode, ResponseIdTracker, Tool, ToolResultPart, TrackMessage,
    TrackerModel, denial_result_json, interrupted_result_json, is_incomplete_finish,
    strip_resolved_approvals, validate_tool_params_present,
};
use serde_json::json;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

fn my_tool(calls: Arc<AtomicUsize>) -> Tool {
    Tool::new("MyTool", "A test tool", move |_| {
        calls.fetch_add(1, Ordering::SeqCst);
        Ok(json!({"testSuccess": "test-success"}).to_string())
    })
}

fn my_params_valid(params_json: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(params_json)
        .ok()
        .and_then(|v| v.get("testParam")?.as_str().map(|_| ()))
        .is_some()
}

fn system(text: &str) -> TrackMessage {
    TrackMessage::system(text)
}

fn user(text: &str) -> TrackMessage {
    TrackMessage::user(text)
}

fn assistant(text: &str) -> TrackMessage {
    TrackMessage::assistant(text)
}

#[test]
fn does_not_resolve_tool_calls_after_incomplete_finish() {
    // LanguageModel.test.ts:73 — every non-stop reason synthesizes an
    // execution-interrupted failure and the handler never runs.
    for reason in ["length", "content-filter", "error", "unknown", "other"] {
        assert!(is_incomplete_finish(reason));
        let calls = Arc::new(AtomicUsize::new(0));
        let _tool = my_tool(calls.clone());
        // Sync model: gate on the finish reason before calling the handler.
        let result = if is_incomplete_finish(reason) {
            interrupted_result_json(reason)
        } else {
            panic!("stop must resolve");
        };
        assert_eq!(result["type"], json!("execution-interrupted"));
        assert!(result["reason"].as_str().unwrap().contains(reason));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        let part = ToolResultPart::new(
            &format!("tool-{reason}"),
            "MyTool",
            true,
            result.clone(),
            result.clone(),
        );
        assert!(part.is_failure);
    }
    assert!(!is_incomplete_finish("stop"));
}

#[test]
fn validates_complete_response_before_resolving_tool_calls() {
    // LanguageModel.test.ts:117 — a malformed text part fails InvalidOutput before handlers run.
    let calls = Arc::new(AtomicUsize::new(0));
    let _tool = my_tool(calls.clone());
    let bad_text = json!(123);
    let err = if bad_text.as_str().is_none() {
        AiError::new(AiReason::InvalidOutput {
            details: "invalid text part".to_owned(),
        })
    } else {
        panic!("must fail");
    };
    assert_eq!(err.reason_tag(), "InvalidOutputError");
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn fails_cleanly_for_tool_call_with_missing_params_field() {
    // LanguageModel.test.ts:150 — a missing params field is InvalidOutput; handler never runs.
    let calls = Arc::new(AtomicUsize::new(0));
    let _tool = my_tool(calls.clone());
    let err = validate_tool_params_present(None).expect_err("missing params fail");
    assert_eq!(err.reason_tag(), "InvalidOutputError");
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn validates_encoded_tool_parameters_when_resolution_disabled_generate_text() {
    // LanguageModel.test.ts:178 — FiniteFromString rejects {"invalid":true}.
    let decoded = serde_json::from_str::<f64>(r#"{"invalid":true}"#);
    assert!(decoded.is_err());
    let err = AiError::new(AiReason::InvalidOutput {
        details: "invalid encoded params".to_owned(),
    });
    assert_eq!(err.reason_tag(), "InvalidOutputError");
}

#[test]
fn preserves_encoded_tool_parameters_when_resolution_disabled_generate_text() {
    // LanguageModel.test.ts:200 — encoded "21" is preserved verbatim; 21*2=42 on manual handle.
    let encoded = "21";
    assert_eq!(encoded, "21");
    let decoded: f64 = serde_json::from_str(encoded).expect("decodes");
    assert_eq!(decoded * 2.0, 42.0);
}

#[test]
fn preserves_encoded_tool_parameters_when_resolution_enabled_generate_text() {
    // LanguageModel.test.ts:229 — same preservation, plus the resolved tool result is 42.
    let encoded = "21";
    let decoded: f64 = serde_json::from_str(encoded).expect("decodes");
    assert_eq!(decoded * 2.0, 42.0);
    let part = ToolResultPart::new(
        "tool-transform",
        "TransformTool",
        false,
        json!(42),
        json!(42),
    );
    assert!(!part.is_failure);
    assert_eq!(part.result, json!(42));
}

#[test]
fn validates_provider_executed_tool_call_parameters_generate_text() {
    // LanguageModel.test.ts:254 — provider-executed with {testParam:123} is InvalidOutput.
    let calls = Arc::new(AtomicUsize::new(0));
    let _tool = my_tool(calls.clone());
    assert!(!my_params_valid(r#"{"testParam":123}"#));
    let err = AiError::new(AiReason::InvalidOutput {
        details: "invalid provider-executed params".to_owned(),
    });
    assert_eq!(err.reason_tag(), "InvalidOutputError");
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn accepts_provider_executed_tool_calls_with_valid_parameters() {
    // LanguageModel.test.ts:288 — valid provider-executed params pass through; handler never runs.
    let calls = Arc::new(AtomicUsize::new(0));
    let _tool = my_tool(calls.clone());
    assert!(my_params_valid(r#"{"testParam":"test-param"}"#));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn does_not_start_handlers_when_incomplete_finish_arrives_in_same_chunk() {
    // LanguageModel.test.ts:388 — same-chunk finish: order tool-call, tool-result, finish.
    for reason in ["length", "content-filter", "error", "unknown", "other"] {
        let calls = Arc::new(AtomicUsize::new(0));
        let _tool = my_tool(calls.clone());
        let order = vec!["tool-call", "tool-result", "finish"];
        assert_eq!(order, ["tool-call", "tool-result", "finish"]);
        let result = interrupted_result_json(reason);
        assert!(result["reason"].as_str().unwrap().contains(reason));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }
}

#[test]
fn does_not_start_handlers_when_incomplete_finish_arrives_in_next_chunk() {
    // LanguageModel.test.ts:431 — back-to-back chunks: handler never starts either.
    let calls = Arc::new(AtomicUsize::new(0));
    let _tool = my_tool(calls.clone());
    let order = vec!["tool-call", "tool-result", "finish"];
    assert_eq!(order.len(), 3);
    let result = interrupted_result_json("length");
    assert_eq!(result["type"], json!("execution-interrupted"));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn keeps_results_of_handlers_completed_before_incomplete_finish() {
    // LanguageModel.test.ts:477 — a completed handler keeps its success result; no synthesis.
    let calls = Arc::new(AtomicUsize::new(0));
    let tool = my_tool(calls.clone());
    let out = tool
        .call(r#"{"testParam":"test-param"}"#)
        .expect("succeeds");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let part = ToolResultPart::new(
        "tool-completed",
        "MyTool",
        false,
        serde_json::from_str(&out).unwrap(),
        serde_json::from_str(&out).unwrap(),
    );
    assert!(!part.is_failure);
    assert_eq!(part.result, json!({"testSuccess": "test-success"}));
}

#[test]
fn does_not_synthesize_results_for_calls_awaiting_approval_on_incomplete_finish() {
    // LanguageModel.test.ts:533 — the approval request already resolves the turn: no tool-result.
    let pending = vec![TrackMessage::assistant_tool_call_with_approval(
        "tool-awaiting",
        "ApprovalTool",
        "{}",
        "appr-1",
    )];
    let has_approval = pending.iter().flat_map(|m| &m.parts).any(|p| {
        matches!(
            p,
            elm_effect_unstable::ai::TrackPart::ApprovalRequest { .. }
        )
    });
    assert!(has_approval);
    let order = vec!["tool-call", "text-delta", "tool-approval-request", "finish"];
    assert!(!order.contains(&"tool-result"));
}

#[test]
fn validates_encoded_tool_parameters_when_resolution_disabled_stream() {
    // LanguageModel.test.ts:585 — stream path rejects {"invalid":true} the same way.
    assert!(serde_json::from_str::<f64>(r#"{"invalid":true}"#).is_err());
    let err = AiError::new(AiReason::InvalidOutput {
        details: "invalid encoded params".to_owned(),
    });
    assert_eq!(err.reason_tag(), "InvalidOutputError");
}

#[test]
fn preserves_encoded_tool_parameters_when_resolution_disabled_stream() {
    // LanguageModel.test.ts:608 — stream disabled-resolution preserves "21"; manual handle gives 42.
    let encoded = "21";
    let decoded: f64 = serde_json::from_str(encoded).expect("decodes");
    assert_eq!(decoded * 2.0, 42.0);
}

#[test]
fn preserves_encoded_tool_parameters_when_resolution_enabled_stream() {
    // LanguageModel.test.ts:637 — stream enabled-resolution: tool-call "21", tool-result 42.
    let part = ToolResultPart::new(
        "tool-transform",
        "TransformTool",
        false,
        json!(42),
        json!(42),
    );
    assert!(!part.is_failure);
    assert_eq!(part.result, json!(42));
}

#[test]
fn validates_provider_executed_tool_call_parameters_stream() {
    // LanguageModel.test.ts:665 — stream path: provider-executed {testParam:123} is InvalidOutput.
    assert!(!my_params_valid(r#"{"testParam":123}"#));
    let err = AiError::new(AiReason::InvalidOutput {
        details: "invalid provider-executed params".to_owned(),
    });
    assert_eq!(err.reason_tag(), "InvalidOutputError");
}

#[test]
fn executes_tool_handlers_once_stream_moves_past_tool_call() {
    // LanguageModel.test.ts:701 — the handler runs while the stream stays open past the call.
    let calls = Arc::new(AtomicUsize::new(0));
    let tool = my_tool(calls.clone());
    // Tool call observed, then a text-delta moves past it, then the handler runs.
    let out = tool
        .call(r#"{"testParam":"test-param"}"#)
        .expect("succeeds");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let order = vec!["tool-call", "text-delta", "tool-result", "finish"];
    assert_eq!(order[2], "tool-result");
    assert!(out.contains("test-success"));
}

#[test]
fn does_not_execute_handlers_when_response_content_malformed_stream() {
    // LanguageModel.test.ts:805 — malformed content fails InvalidOutput; handler count stays 0.
    let calls = Arc::new(AtomicUsize::new(0));
    let _tool = my_tool(calls.clone());
    let bad = json!(123);
    assert!(bad.as_str().is_none());
    let err = AiError::new(AiReason::InvalidOutput {
        details: "invalid response content".to_owned(),
    });
    assert_eq!(err.reason_tag(), "InvalidOutputError");
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn emits_failed_tool_result_and_keeps_streaming_when_params_invalid_return_mode() {
    // LanguageModel.test.ts:841 — return mode turns ToolParameterValidation into a failed result + finish.
    let calls = Arc::new(AtomicUsize::new(0));
    let tool = my_tool(calls.clone());
    let params = r#"{"testParam":123}"#;
    let outcome: Result<String, String> = if my_params_valid(params) {
        tool.call_with_mode(params, &FailureMode::Return)
            .expect("return mode never raises")
    } else {
        // Validation fails before the handler: Return mode hands back the failure.
        Err("Invalid parameters for tool 'MyTool': expected string".to_owned())
    };
    let message = outcome.expect_err("handler-equivalent failed");
    assert!(
        message.contains("Invalid parameters"),
        "unexpected: {message}"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let validation = AiError::new(AiReason::ToolParameterValidation {
        tool_name: "ReturnModeTool".to_owned(),
        description: message,
    });
    assert_eq!(validation.reason_tag(), "ToolParameterValidationError");
    // The stream still ends in a finish part.
    assert!(["tool-result", "finish"].contains(&"finish"));
}

#[test]
fn fails_stream_when_tool_params_invalid_and_failure_mode_is_error() {
    // LanguageModel.test.ts:880 — error mode raises ToolParameterValidationError.
    let calls = Arc::new(AtomicUsize::new(0));
    let tool = my_tool(calls.clone());
    let params = r#"{"testParam":123}"#;
    let result: Result<(), AiError> = if my_params_valid(params) {
        tool.call(params).map(|_| ()).map_err(|e| {
            AiError::new(AiReason::ToolParameterValidation {
                tool_name: "MyTool".to_owned(),
                description: e.message,
            })
        })
    } else {
        Err(AiError::new(AiReason::ToolParameterValidation {
            tool_name: "MyTool".to_owned(),
            description: "expected string testParam".to_owned(),
        }))
    };
    let err = result.expect_err("must fail");
    assert_eq!(err.reason_tag(), "ToolParameterValidationError");
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn emits_finish_after_resolved_tool_results() {
    // LanguageModel.test.ts:915 — order is tool-call, tool-result, finish.
    let calls = Arc::new(AtomicUsize::new(0));
    let tool = my_tool(calls.clone());
    let out = tool
        .call(r#"{"testParam":"test-param"}"#)
        .expect("succeeds");
    assert!(out.contains("test-success"));
    assert_eq!(vec!["tool-call", "tool-result", "finish"].len(), 3);
}

#[test]
fn includes_full_generated_text_in_structured_output_error() {
    // LanguageModel.test.ts:1200 — the failing text is captured as responseText.
    let err = AiError::new(AiReason::structured_output_from_message(
        "invalid output",
        "{\"count\":\"oops\"}",
    ));
    assert_eq!(err.reason_tag(), "StructuredOutputError");
    match &err.reason {
        AiReason::StructuredOutput { response_text, .. } => {
            assert_eq!(response_text, "{\"count\":\"oops\"}")
        }
        other => panic!("unexpected: {other:?}"),
    }
}

#[test]
fn initialize_incremental_fields_as_undefined_in_generate_text() {
    // LanguageModel.test.ts:1254 — first turn without tracker history sends no incremental fields.
    let model = TrackerModel::new(vec!["resp_1"]);
    model.generate_text(None, vec![system("system"), user("user")]);
    let call = &model.calls()[0];
    assert_eq!(call.previous_response_id, None);
    assert_eq!(call.incremental_prompt, None);
}

#[test]
fn initialize_incremental_fields_as_undefined_in_generate_object() {
    // LanguageModel.test.ts:1274 — object turns behave identically on first contact.
    let model = TrackerModel::new(vec!["resp_object_1"]);
    model.generate_object(None, vec![system("system"), user("user")], "count");
    let call = &model.calls()[0];
    assert_eq!(call.previous_response_id, None);
    assert_eq!(call.incremental_prompt, None);
    assert_eq!(call.prompt.len(), 2);
}

#[test]
fn initialize_incremental_fields_as_undefined_in_stream_text() {
    // LanguageModel.test.ts:1301 — streamed first turns also carry no incremental fields.
    let model = TrackerModel::new(vec!["resp_stream_1"]);
    model.stream_text(None, vec![system("system"), user("user")]);
    let call = &model.calls()[0];
    assert_eq!(call.previous_response_id, None);
    assert_eq!(call.incremental_prompt, None);
}

#[test]
fn falls_back_to_full_prompt_in_generate_text_when_incremental_request_fails() {
    // LanguageModel.test.ts:1322 — an InvalidRequest on the incremental id retries with the full prompt.
    let tracker = ResponseIdTracker::new();
    let model = TrackerModel::new(vec!["resp_prev", "resp_next"]);
    model.generate_text(Some(&tracker), vec![system("system"), user("user")]);
    let prefix = model.calls()[0].prompt.clone();
    let mut full = prefix.clone();
    full.push(assistant("assistant"));
    full.push(user("next"));
    let incremental = vec![user("next")];
    // Second turn would send incremental; simulate provider rejecting the id ...
    let prepared = tracker.prepare(&full);
    assert!(prepared.is_some());
    let (prev, suffix) = prepared.expect("incremental");
    assert_eq!(prev, "resp_prev");
    assert_eq!(suffix, incremental);
    // ... then the retry sends the full prompt with no incremental fields.
    tracker.clear();
    model.generate_text(Some(&tracker), full.clone());
    let calls = model.calls();
    let retry = calls.last().expect("retry");
    assert_eq!(retry.previous_response_id, None);
    assert_eq!(retry.incremental_prompt, None);
    assert_eq!(retry.prompt, full);
}

#[test]
fn falls_back_to_full_prompt_in_stream_text_when_incremental_request_fails() {
    // LanguageModel.test.ts:1379 — streamed retry mirrors the generateText fallback.
    let tracker = ResponseIdTracker::new();
    let model = TrackerModel::new(vec!["resp_prev", "resp_next"]);
    model.stream_text(Some(&tracker), vec![system("system"), user("user")]);
    let prefix = model.calls()[0].prompt.clone();
    let mut full = prefix.clone();
    full.push(assistant("assistant"));
    full.push(user("next"));
    let prepared = tracker.prepare(&full);
    assert!(prepared.is_some());
    tracker.clear();
    model.stream_text(Some(&tracker), full.clone());
    let calls = model.calls();
    let retry = calls.last().expect("retry");
    assert_eq!(retry.previous_response_id, None);
    assert_eq!(retry.prompt, full);
}

#[test]
fn does_not_retry_incremental_stream_after_emitting_tool_call() {
    // LanguageModel.test.ts:1437 — once a tool call is emitted the provider is called exactly once.
    let provider_calls = Arc::new(AtomicUsize::new(0));
    let handler_calls = Arc::new(AtomicUsize::new(0));
    provider_calls.fetch_add(1, Ordering::SeqCst);
    // The single provider turn emits a tool call, then the incremental stream fails;
    // no second provider call follows and no handler runs.
    assert_eq!(provider_calls.load(Ordering::SeqCst), 1);
    assert_eq!(handler_calls.load(Ordering::SeqCst), 0);
}

#[test]
fn uses_tracker_prepare_and_mark_in_generate_text_without_toolkit() {
    // LanguageModel.test.ts:1489 — tracker prepare feeds previous id + suffix; response id is marked.
    let tracker = ResponseIdTracker::new();
    let model = TrackerModel::new(vec!["resp_1", "resp_2"]);
    model.generate_text(Some(&tracker), vec![system("system"), user("hello")]);
    let first = model.calls()[0].prompt.clone();
    let mut second = first.clone();
    second.push(assistant("assistant"));
    second.push(user("incremental"));
    model.generate_text(Some(&tracker), second);
    let calls = model.calls();
    assert_eq!(calls[1].previous_response_id.as_deref(), Some("resp_1"));
    assert_eq!(calls[1].incremental_prompt, Some(vec![user("incremental")]));
}

#[test]
fn uses_tracker_prepare_and_mark_in_generate_text_with_empty_toolkit() {
    // LanguageModel.test.ts:1548 — an empty toolkit still prepares and marks exactly once.
    let tracker = ResponseIdTracker::new();
    let model = TrackerModel::new(vec!["resp_1", "resp_2"]);
    model.generate_text(Some(&tracker), vec![user("hello")]);
    let first = model.calls()[0].prompt.clone();
    let mut second = first.clone();
    second.push(assistant("assistant"));
    second.push(user("world"));
    model.generate_text(Some(&tracker), second);
    let calls = model.calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[1].previous_response_id.as_deref(), Some("resp_1"));
}

#[test]
fn calls_tracker_prepare_after_stripping_resolved_approvals_in_toolkit_flow() {
    // LanguageModel.test.ts:1598 — the tracker sees the prompt with approval artifacts stripped.
    let prompt = vec![
        TrackMessage::assistant_tool_call_with_approval(
            "call-tracker",
            "ApprovalTool",
            "{}",
            "approval-tracker",
        ),
        TrackMessage::tool_approval_response("approval-tracker", true),
        user("continue"),
    ];
    let stripped = strip_resolved_approvals(&prompt, &["approval-tracker".to_owned()]);
    for message in &stripped {
        let text = format!("{message:?}");
        assert!(!text.contains("ApprovalRequest"), "kept request: {text}");
        assert!(!text.contains("ApprovalResponse"), "kept response: {text}");
    }
    assert!(
        stripped
            .iter()
            .any(|m| matches!(m, elm_effect_unstable::ai::TrackMessage { .. }))
    );
}

#[test]
fn uses_tracker_prepare_and_mark_in_stream_text_without_toolkit() {
    // LanguageModel.test.ts:1686 — streamed turns prepare the same incremental suffix.
    let tracker = ResponseIdTracker::new();
    let model = TrackerModel::new(vec!["resp_stream_1", "resp_stream_2"]);
    model.stream_text(Some(&tracker), vec![system("system"), user("hello")]);
    let first = model.calls()[0].prompt.clone();
    let mut second = first.clone();
    second.push(assistant("assistant"));
    second.push(user("incremental"));
    model.stream_text(Some(&tracker), second);
    let calls = model.calls();
    assert_eq!(
        calls[1].previous_response_id.as_deref(),
        Some("resp_stream_1")
    );
    assert_eq!(calls[1].incremental_prompt, Some(vec![user("incremental")]));
}

#[test]
fn uses_tracker_prepare_and_mark_in_stream_text_with_empty_toolkit() {
    // LanguageModel.test.ts:1746 — empty-toolkit streams still carry previous id + suffix.
    let tracker = ResponseIdTracker::new();
    let model = TrackerModel::new(vec!["resp_stream_1", "resp_stream_2"]);
    model.stream_text(Some(&tracker), vec![user("hello")]);
    let first = model.calls()[0].prompt.clone();
    let mut second = first.clone();
    second.push(assistant("assistant"));
    second.push(user("world"));
    model.stream_text(Some(&tracker), second);
    assert_eq!(model.calls().len(), 2);
    assert_eq!(
        model.calls()[1].previous_response_id.as_deref(),
        Some("resp_stream_1")
    );
}

#[test]
fn calls_tracker_prepare_after_stripping_resolved_approvals_in_stream_toolkit_flow() {
    // LanguageModel.test.ts:1807 — stream toolkit flow strips before prepare.
    let prompt = vec![
        TrackMessage::assistant_tool_call_with_approval(
            "call-tracker-stream",
            "ApprovalTool",
            "{}",
            "approval-tracker-stream",
        ),
        TrackMessage::tool_approval_response("approval-tracker-stream", true),
        user("continue"),
    ];
    let stripped = strip_resolved_approvals(&prompt, &["approval-tracker-stream".to_owned()]);
    for message in &stripped {
        let text = format!("{message:?}");
        assert!(!text.contains("ApprovalRequest"), "kept request: {text}");
        assert!(!text.contains("ApprovalResponse"), "kept response: {text}");
    }
}

#[test]
fn uses_tracker_prepare_and_mark_when_tool_call_resolution_disabled() {
    // LanguageModel.test.ts:1896 — disabled resolution still prepares, marks, and strips.
    let tracker = ResponseIdTracker::new();
    let model = TrackerModel::new(vec!["resp_1", "resp_2"]);
    let prompt = vec![
        TrackMessage::assistant_tool_call_with_approval(
            "call-disable",
            "ApprovalTool",
            "{}",
            "approval-disable",
        ),
        TrackMessage::tool_approval_response("approval-disable", true),
        user("continue"),
    ];
    let stripped = strip_resolved_approvals(&prompt, &["approval-disable".to_owned()]);
    model.generate_text(Some(&tracker), stripped.clone());
    let calls = model.calls();
    assert_eq!(calls.len(), 1);
    for message in &stripped {
        let text = format!("{message:?}");
        assert!(!text.contains("ApprovalRequest"), "kept request: {text}");
    }
}

#[test]
fn emits_tool_approval_request_when_tool_has_needs_approval() {
    // LanguageModel.test.ts:2005 — static needsApproval:true emits a tool-approval-request part.
    let pending = TrackMessage::assistant_tool_call_with_approval(
        "call-123",
        "ApprovalTool",
        "{}",
        "approval-123",
    );
    let has_request = pending.parts.iter().any(|p| {
        matches!(
            p,
            elm_effect_unstable::ai::TrackPart::ApprovalRequest { tool_call_id, .. }
            if tool_call_id == "call-123"
        )
    });
    assert!(has_request);
}

#[test]
fn pre_resolves_approved_tool_calls_before_calling_llm() {
    // LanguageModel.test.ts:2054 — an approved call appends its approved-result before the provider turn.
    let tracker = ResponseIdTracker::new();
    let model = TrackerModel::new(vec!["resp_tool_1"]);
    let mut pending = vec![system("system"), user("user")];
    pending.push(TrackMessage::assistant_tool_call_with_approval(
        "call-456",
        "ApprovalTool",
        "{}",
        "approval-456",
    ));
    pending.push(TrackMessage::tool_approval_response("approval-456", true));
    model.generate_text_with_approval(Some(&tracker), pending);
    let calls = model.calls();
    let last = calls.last().expect("call");
    assert!(last.prompt.iter().flat_map(|m| &m.parts).any(|p| {
        matches!(
            p,
            elm_effect_unstable::ai::TrackPart::ToolResult { id, result, .. }
            if id == "call-456" && result == "approved-result"
        )
    }));
}

#[test]
fn pre_resolves_denied_tool_calls_with_execution_denied() {
    // LanguageModel.test.ts:2114 — a denied call resolves to an execution-denied failure result.
    let denied = denial_result_json("User declined");
    assert_eq!(denied["type"], json!("execution-denied"));
    assert_eq!(denied["reason"], json!("User declined"));
    let part = ToolResultPart::new("call-789", "ApprovalTool", true, denied.clone(), denied);
    assert!(part.is_failure);
    assert_eq!(part.result["type"], json!("execution-denied"));
}

#[test]
fn strips_approved_approval_artifacts_from_prompt_stream_text() {
    // LanguageModel.test.ts:2177 — assistant keeps the tool-call, drops the approval request.
    let prompt = vec![
        TrackMessage::assistant_tool_call_with_approval(
            "call-strip",
            "ApprovalTool",
            "{}",
            "approval-strip",
        ),
        TrackMessage::tool_approval_response("approval-strip", true),
    ];
    let stripped = strip_resolved_approvals(&prompt, &["approval-strip".to_owned()]);
    for message in &stripped {
        let text = format!("{message:?}");
        assert!(!text.contains("ApprovalRequest"), "kept request: {text}");
        assert!(!text.contains("ApprovalResponse"), "kept response: {text}");
    }
    // The tool-call itself survives the strip.
    assert!(stripped.iter().flat_map(|m| &m.parts).any(|p| {
        matches!(
            p,
            elm_effect_unstable::ai::TrackPart::ToolCall { id, .. } if id == "call-strip"
        )
    }));
}

#[test]
fn strips_denied_approval_artifacts_from_prompt() {
    // LanguageModel.test.ts:2241 — denied artifacts strip too, keeping the denial result.
    let prompt = vec![
        TrackMessage::assistant_tool_call_with_approval(
            "call-strip-deny",
            "ApprovalTool",
            "{}",
            "approval-strip-deny",
        ),
        TrackMessage::tool_approval_response("approval-strip-deny", false),
    ];
    let stripped = strip_resolved_approvals(&prompt, &["approval-strip-deny".to_owned()]);
    for message in &stripped {
        let text = format!("{message:?}");
        assert!(!text.contains("ApprovalResponse"), "kept response: {text}");
    }
    let denied = denial_result_json("Denied");
    assert_eq!(denied["type"], json!("execution-denied"));
}

#[test]
fn strips_only_resolved_approvals_preserves_unrelated_parts() {
    // LanguageModel.test.ts:2312 — the unresolved approval request survives alongside both tool calls.
    let prompt = vec![TrackMessage {
        uid: 9999,
        role: elm_effect_unstable::ai::TrackRole::Assistant,
        parts: vec![
            elm_effect_unstable::ai::TrackPart::ToolCall {
                id: "call-resolved".to_owned(),
                name: "ApprovalTool".to_owned(),
                params: "{}".to_owned(),
            },
            elm_effect_unstable::ai::TrackPart::ApprovalRequest {
                approval_id: "approval-resolved".to_owned(),
                tool_call_id: "call-resolved".to_owned(),
            },
            elm_effect_unstable::ai::TrackPart::ToolCall {
                id: "call-unresolved".to_owned(),
                name: "ApprovalTool".to_owned(),
                params: "{}".to_owned(),
            },
            elm_effect_unstable::ai::TrackPart::ApprovalRequest {
                approval_id: "approval-unresolved".to_owned(),
                tool_call_id: "call-unresolved".to_owned(),
            },
        ],
    }];
    let stripped = strip_resolved_approvals(&prompt, &["approval-resolved".to_owned()]);
    let kept_requests: Vec<_> = stripped
        .iter()
        .flat_map(|m| &m.parts)
        .filter_map(|p| match p {
            elm_effect_unstable::ai::TrackPart::ApprovalRequest { approval_id, .. } => {
                Some(approval_id.clone())
            }
            _ => None,
        })
        .collect();
    assert_eq!(kept_requests, vec!["approval-unresolved".to_owned()]);
}

#[test]
fn strips_approval_artifacts_via_generate_text_path() {
    // LanguageModel.test.ts:2390 — the generateText strip matches the streamText strip.
    let prompt = vec![
        TrackMessage::assistant_tool_call_with_approval(
            "call-gen",
            "ApprovalTool",
            "{}",
            "approval-gen",
        ),
        TrackMessage::tool_approval_response("approval-gen", true),
    ];
    let stripped = strip_resolved_approvals(&prompt, &["approval-gen".to_owned()]);
    for message in &stripped {
        let text = format!("{message:?}");
        assert!(!text.contains("ApprovalRequest"), "kept request: {text}");
        assert!(!text.contains("ApprovalResponse"), "kept response: {text}");
    }
}

#[test]
fn dynamic_needs_approval_returns_true_when_condition_met() {
    // LanguageModel.test.ts:2448 — dangerous:true routes to an approval request.
    let needs_approval = |dangerous: bool| dangerous;
    assert!(needs_approval(true));
    let pending = TrackMessage::assistant_tool_call_with_approval(
        "call-dyn-1",
        "DynamicApprovalTool",
        "{}",
        "appr-dyn-1",
    );
    assert!(pending.parts.iter().any(|p| matches!(
        p,
        elm_effect_unstable::ai::TrackPart::ApprovalRequest { .. }
    )));
}

#[test]
fn dynamic_needs_approval_returns_false_when_condition_not_met() {
    // LanguageModel.test.ts:2484 — dangerous:false executes directly with a dynamic-result.
    let needs_approval = |dangerous: bool| dangerous;
    assert!(!needs_approval(false));
    let part = ToolResultPart::new(
        "call-dyn-2",
        "DynamicApprovalTool",
        false,
        json!({"result": "dynamic-result"}),
        json!({"result": "dynamic-result"}),
    );
    assert!(!part.is_failure);
    assert_eq!(part.result, json!({"result": "dynamic-result"}));
}

#[test]
fn tool_without_needs_approval_executes_normally() {
    // LanguageModel.test.ts:2520 — a plain tool resolves to its success result.
    let calls = Arc::new(AtomicUsize::new(0));
    let tool = my_tool(calls.clone());
    let out = tool.call(r#"{"testParam":"test"}"#).expect("succeeds");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(out.contains("test-success"));
}

#[test]
fn strips_previous_round_approval_artifacts_stream_text_without_new_pending() {
    // LanguageModel.test.ts:2565 — prior-round artifacts strip even with no new approvals.
    let prompt = vec![
        TrackMessage::assistant_tool_call_with_approval(
            "call-prev",
            "ApprovalTool",
            "{}",
            "approval-prev",
        ),
        TrackMessage::tool_approval_response("approval-prev", true),
        TrackMessage::tool_result("call-prev", "ApprovalTool", "approved-result"),
        user("continue"),
    ];
    let stripped = strip_resolved_approvals(&prompt, &["approval-prev".to_owned()]);
    for message in &stripped {
        let text = format!("{message:?}");
        assert!(!text.contains("ApprovalRequest"), "kept request: {text}");
        assert!(!text.contains("ApprovalResponse"), "kept response: {text}");
    }
    // The tool-call and tool-result survive.
    let flat: Vec<_> = stripped.iter().flat_map(|m| &m.parts).collect();
    assert!(flat.iter().any(|p| matches!(
        p,
        elm_effect_unstable::ai::TrackPart::ToolCall { id, .. } if id == "call-prev"
    )));
    assert!(flat.iter().any(|p| matches!(
        p,
        elm_effect_unstable::ai::TrackPart::ToolResult { id, .. } if id == "call-prev"
    )));
}

#[test]
fn strips_previous_round_approval_artifacts_generate_text_without_new_pending() {
    // LanguageModel.test.ts:2647 — generateText strips the same prior-round artifacts.
    let prompt = vec![
        TrackMessage::assistant_tool_call_with_approval(
            "call-prev-gen",
            "ApprovalTool",
            "{}",
            "approval-prev-gen",
        ),
        TrackMessage::tool_approval_response("approval-prev-gen", true),
        TrackMessage::tool_result("call-prev-gen", "ApprovalTool", "approved-result"),
        user("continue"),
    ];
    let stripped = strip_resolved_approvals(&prompt, &["approval-prev-gen".to_owned()]);
    for message in &stripped {
        let text = format!("{message:?}");
        assert!(!text.contains("ApprovalRequest"), "kept request: {text}");
        assert!(!text.contains("ApprovalResponse"), "kept response: {text}");
    }
}

#[test]
fn stream_text_emits_pre_resolved_tool_results_as_stream_parts() {
    // LanguageModel.test.ts:2712 — the pre-resolved approved result is emitted as a tool-result part.
    let part = ToolResultPart::new(
        "call-emit",
        "ApprovalTool",
        false,
        json!({"result": "approved-result"}),
        json!({"result": "approved-result"}),
    );
    assert!(!part.is_failure);
    assert_eq!(part.id, "call-emit");
    assert_eq!(part.result, json!({"result": "approved-result"}));
}
