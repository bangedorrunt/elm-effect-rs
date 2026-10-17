//! `packages/effect/test/ai/Prompt.test.ts` assembly witnesses.

use elm_effect_unstable::prompt::{
    Prompt, PromptMessage, PromptPart, ResponsePart, Role, from_response_parts,
};
use serde_json::{Map, Value};

fn options(pairs: &[(&str, Value)]) -> Map<String, Value> {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect()
}

fn text_part(text: &str) -> PromptPart {
    PromptPart::Text {
        text: text.to_owned(),
        options: Map::new(),
    }
}

#[test]
fn preserves_text_parts_during_serialization() {
    // Prompt.test.ts:7 — the prompt survives a JSON round trip unchanged.
    let prompt = Prompt::make(vec![
        PromptMessage::new(Role::User, vec![text_part("first"), text_part("second")]),
        PromptMessage::new(
            Role::Assistant,
            vec![PromptPart::Text {
                text: "response".to_owned(),
                options: options(&[("test", serde_json::json!({"cache": "ephemeral"}))]),
            }],
        ),
    ]);
    let wire = serde_json::to_value(&prompt_as_json(&prompt)).expect("encodes");
    let back = prompt_from_json(&wire).expect("decodes");
    assert_eq!(back, prompt);
}

#[test]
fn exports_schemas_for_all_parts_and_message_specific_parts() {
    // Prompt.test.ts:30 — part-kind membership per role.
    let text = text_part("hello");
    let reasoning = PromptPart::Reasoning {
        text: "thinking".into(),
        options: Map::new(),
    };
    let file = PromptPart::File {
        media_type: "text/plain".into(),
        data_base64: "aGVsbG8=".into(),
        options: Map::new(),
    };
    let tool_call = PromptPart::ToolCall {
        id: "call-1".into(),
        name: "tool".into(),
        params: serde_json::json!({}),
        provider_executed: false,
        options: Map::new(),
    };
    let tool_result = PromptPart::ToolResult {
        id: "call-1".into(),
        name: "tool".into(),
        is_failure: false,
        result: Value::String("done".into()),
        provider_executed: false,
        preliminary: false,
        options: Map::new(),
    };
    let approval_request = PromptPart::ToolApprovalRequest {
        approval_id: "approval-1".into(),
        tool_call_id: "call-1".into(),
        options: Map::new(),
    };
    let approval_response = PromptPart::ToolApprovalResponse {
        approval_id: "approval-1".into(),
        approved: true,
        options: Map::new(),
    };
    use Role::*;
    let user = [User];
    let assistant = [Assistant];
    let tool = [Tool];
    assert!(
        text.allowed_roles()
            .iter()
            .all(|role| user.contains(role) || assistant.contains(role))
    );
    assert!(
        file.allowed_roles()
            .iter()
            .all(|role| user.contains(role) || assistant.contains(role))
    );
    for part in [&reasoning, &tool_call, &approval_request] {
        assert_eq!(part.allowed_roles(), &assistant);
    }
    for part in [&tool_result, &approval_response] {
        assert_eq!(part.allowed_roles(), &tool);
    }
    assert!(!text.allowed_roles().contains(&Tool));
    assert!(!tool_result.allowed_roles().contains(&User));
}

#[test]
fn preserves_file_only_responses_as_assistant_messages() {
    // Prompt.test.ts:81 — bytes travel as base64 with metadata as options.
    let prompt = from_response_parts(vec![ResponsePart::File {
        data: vec![0, 127, 128, 255],
        media_type: "application/octet-stream".into(),
        metadata: options(&[(
            "example",
            serde_json::json!({"id": "attachment-1", "generated": true}),
        )]),
    }]);
    assert_eq!(
        prompt,
        Prompt::make(vec![PromptMessage::new(
            Role::Assistant,
            vec![PromptPart::File {
                media_type: "application/octet-stream".into(),
                data_base64: "AH+A/w==".into(),
                options: options(&[(
                    "example",
                    serde_json::json!({"id": "attachment-1", "generated": true})
                )]),
            }]
        )])
    );
}

#[test]
fn folds_streamed_text_and_reasoning_deltas_into_an_assistant_message() {
    // Prompt.test.ts:102
    let prompt = from_response_parts(vec![
        ResponsePart::TextStart {
            id: "1".into(),
            metadata: Map::new(),
        },
        ResponsePart::TextDelta {
            id: "1".into(),
            delta: "Hello".into(),
            metadata: Map::new(),
        },
        ResponsePart::TextDelta {
            id: "1".into(),
            delta: ", ".into(),
            metadata: Map::new(),
        },
        ResponsePart::TextDelta {
            id: "1".into(),
            delta: "World!".into(),
            metadata: Map::new(),
        },
        ResponsePart::TextEnd {
            id: "1".into(),
            metadata: Map::new(),
        },
        ResponsePart::ReasoningStart {
            id: "2".into(),
            metadata: Map::new(),
        },
        ResponsePart::ReasoningDelta {
            id: "2".into(),
            delta: "I ".into(),
            metadata: Map::new(),
        },
        ResponsePart::ReasoningDelta {
            id: "2".into(),
            delta: "am ".into(),
            metadata: Map::new(),
        },
        ResponsePart::ReasoningDelta {
            id: "2".into(),
            delta: "thinking".into(),
            metadata: Map::new(),
        },
        ResponsePart::ReasoningEnd {
            id: "2".into(),
            metadata: Map::new(),
        },
    ]);
    assert_eq!(
        prompt,
        Prompt::make(vec![PromptMessage::new(
            Role::Assistant,
            vec![
                PromptPart::Text {
                    text: "Hello, World!".into(),
                    options: Map::new()
                },
                PromptPart::Reasoning {
                    text: "I am thinking".into(),
                    options: Map::new()
                },
            ]
        )])
    );
}

#[test]
fn preserves_metadata_on_non_streaming_response_parts() {
    // Prompt.test.ts:128
    let prompt = from_response_parts(vec![
        ResponsePart::Text {
            text: "Hello".into(),
            metadata: options(&[("test", serde_json::json!({"value": "text"}))]),
        },
        ResponsePart::Reasoning {
            text: "Thinking".into(),
            metadata: options(&[(
                "openai",
                serde_json::json!({"encryptedContent": "encrypted-reasoning"}),
            )]),
        },
        ResponsePart::ToolCall {
            id: "call-1".into(),
            name: "get_weather".into(),
            params: serde_json::json!({"city": "London"}),
            provider_executed: false,
            metadata: options(&[(
                "google",
                serde_json::json!({"thoughtSignature": "signed-call"}),
            )]),
        },
        ResponsePart::ToolResult {
            id: "call-1".into(),
            name: "get_weather".into(),
            is_failure: false,
            result: serde_json::json!({"temp": 20}),
            preliminary: false,
            provider_executed: false,
            metadata: options(&[("test", serde_json::json!({"value": "tool-result"}))]),
        },
    ]);
    assert_eq!(prompt.messages.len(), 2);
    assert_eq!(prompt.messages[0].role, Role::Assistant);
    assert_eq!(prompt.messages[0].parts.len(), 3);
    assert_eq!(prompt.messages[1].role, Role::Tool);
    assert_eq!(prompt.messages[1].parts.len(), 1);
    match &prompt.messages[1].parts[0] {
        PromptPart::ToolResult { id, options, .. } => {
            assert_eq!(id, "call-1");
            assert_eq!(
                options.get("test"),
                Some(&serde_json::json!({"value": "tool-result"}))
            );
        }
        other => panic!("expected a tool result, got {other:?}"),
    }
}

#[test]
fn accumulates_metadata_across_streamed_text_and_reasoning_parts() {
    // Prompt.test.ts:197 — option maps merge across the stream's lifetime.
    let prompt = from_response_parts(vec![
        ResponsePart::TextStart {
            id: "text-1".into(),
            metadata: options(&[("testStart", serde_json::json!({"value": "text-start"}))]),
        },
        ResponsePart::TextDelta {
            id: "text-1".into(),
            delta: "Hello".into(),
            metadata: options(&[("testDelta", serde_json::json!({"value": "text-delta"}))]),
        },
        ResponsePart::TextEnd {
            id: "text-1".into(),
            metadata: options(&[("testEnd", serde_json::json!({"value": "text-end"}))]),
        },
        ResponsePart::ReasoningStart {
            id: "reasoning-1".into(),
            metadata: options(&[("openai", serde_json::json!({"itemId": "reasoning-item"}))]),
        },
        ResponsePart::ReasoningDelta {
            id: "reasoning-1".into(),
            delta: "Thinking".into(),
            metadata: Map::new(),
        },
        ResponsePart::ReasoningDelta {
            id: "reasoning-1".into(),
            delta: String::new(),
            metadata: options(&[("testDelta", serde_json::json!({"value": "reasoning-delta"}))]),
        },
        ResponsePart::ReasoningEnd {
            id: "reasoning-1".into(),
            metadata: options(&[
                (
                    "openai",
                    serde_json::json!({"encryptedContent": "encrypted-reasoning"}),
                ),
                ("testEnd", serde_json::json!({"value": "reasoning-end"})),
            ]),
        },
    ]);
    assert_eq!(prompt.messages.len(), 1);
    match &prompt.messages[0].parts[..] {
        [
            PromptPart::Text { text, options },
            PromptPart::Reasoning {
                text: reasoning,
                options: reasoning_options,
            },
        ] => {
            assert_eq!(text, "Hello");
            assert_eq!(options.len(), 3);
            assert_eq!(reasoning, "Thinking");
            assert_eq!(
                reasoning_options.get("openai"),
                Some(
                    &serde_json::json!({"itemId": "reasoning-item", "encryptedContent": "encrypted-reasoning"})
                )
            );
        }
        other => panic!("expected folded text and reasoning, got {other:?}"),
    }
}

fn tool_call(id: &str, name: &str, provider_executed: bool) -> ResponsePart {
    ResponsePart::ToolCall {
        id: id.into(),
        name: name.into(),
        params: serde_json::json!({}),
        provider_executed,
        metadata: Map::new(),
    }
}

fn tool_result(
    id: &str,
    name: &str,
    result: Value,
    preliminary: bool,
    provider_executed: bool,
) -> ResponsePart {
    ResponsePart::ToolResult {
        id: id.into(),
        name: name.into(),
        is_failure: false,
        result,
        preliminary,
        provider_executed,
        metadata: Map::new(),
    }
}

#[test]
fn places_tool_calls_in_assistant_messages_and_tool_results_in_tool_messages() {
    // Prompt.test.ts:267
    let prompt = from_response_parts(vec![
        tool_call("call-1", "get_weather", false),
        tool_result(
            "call-1",
            "get_weather",
            serde_json::json!({"temp": 20}),
            false,
            false,
        ),
    ]);
    assert_eq!(prompt.messages.len(), 2);
    assert_eq!(prompt.messages[0].role, Role::Assistant);
    assert_eq!(prompt.messages[1].role, Role::Tool);
    assert!(matches!(
        prompt.messages[0].parts[0],
        PromptPart::ToolCall { .. }
    ));
    assert!(matches!(
        prompt.messages[1].parts[0],
        PromptPart::ToolResult { .. }
    ));
}

#[test]
fn places_provider_executed_tool_results_in_the_assistant_message() {
    // Prompt.test.ts:303 — the provider-executed pair never forms a tool message.
    let prompt = from_response_parts(vec![
        tool_call("ws-1", "web_search", true),
        tool_result(
            "ws-1",
            "web_search",
            serde_json::json!({"sources": 3}),
            false,
            true,
        ),
        tool_call("call-1", "get_weather", false),
        tool_result(
            "call-1",
            "get_weather",
            serde_json::json!({"temp": 20}),
            false,
            false,
        ),
    ]);
    assert_eq!(prompt.messages.len(), 2);
    assert_eq!(prompt.messages[0].parts.len(), 3);
    match &prompt.messages[0].parts[1] {
        PromptPart::ToolResult {
            id,
            provider_executed,
            ..
        } => {
            assert_eq!(id, "ws-1");
            assert!(*provider_executed);
        }
        other => panic!("expected the provider-executed result second, got {other:?}"),
    }
    assert_eq!(prompt.messages[1].parts.len(), 1);
}

#[test]
fn handles_out_of_order_tool_results() {
    // Prompt.test.ts:356 — calls keep arrival order, results keep arrival order.
    let prompt = from_response_parts(vec![
        tool_call("call-A", "tool_a", false),
        tool_call("call-B", "tool_b", false),
        tool_result(
            "call-B",
            "tool_b",
            Value::String("result-B".into()),
            false,
            false,
        ),
        tool_result(
            "call-A",
            "tool_a",
            Value::String("result-A".into()),
            false,
            false,
        ),
    ]);
    assert_eq!(prompt.messages.len(), 2);
    let call_ids: Vec<&str> = prompt.messages[0]
        .parts
        .iter()
        .map(|part| match part {
            PromptPart::ToolCall { id, .. } => id.as_str(),
            other => panic!("expected tool calls, got {other:?}"),
        })
        .collect();
    assert_eq!(call_ids, vec!["call-A", "call-B"]);
    let result_ids: Vec<&str> = prompt.messages[1]
        .parts
        .iter()
        .map(|part| match part {
            PromptPart::ToolResult { id, .. } => id.as_str(),
            other => panic!("expected tool results, got {other:?}"),
        })
        .collect();
    assert_eq!(result_ids, vec!["call-B", "call-A"]);
}

#[test]
fn keeps_only_the_final_non_preliminary_tool_result() {
    // Prompt.test.ts:418 — preliminary progress updates never reach the prompt.
    let prompt = from_response_parts(vec![
        tool_call("call-1", "long_task", false),
        tool_result(
            "call-1",
            "long_task",
            serde_json::json!({"progress": 50}),
            true,
            false,
        ),
        tool_result(
            "call-1",
            "long_task",
            serde_json::json!({"progress": 100}),
            true,
            false,
        ),
        tool_result(
            "call-1",
            "long_task",
            serde_json::json!({"done": true}),
            false,
            false,
        ),
    ]);
    assert_eq!(prompt.messages.len(), 2);
    assert_eq!(prompt.messages[1].parts.len(), 1);
    match &prompt.messages[1].parts[0] {
        PromptPart::ToolResult { result, .. } => {
            assert_eq!(*result, serde_json::json!({"done": true}));
        }
        other => panic!("expected the final result, got {other:?}"),
    }
}

#[test]
fn sequentially_combines_the_content_of_two_prompts() {
    // Prompt.test.ts:470
    let left = Prompt::make(vec![
        PromptMessage::text(Role::User, "a"),
        PromptMessage::text(Role::Assistant, "b"),
    ]);
    let right = vec![
        PromptMessage::text(Role::User, "c"),
        PromptMessage::text(Role::Assistant, "d"),
    ];
    let merged = Prompt::concat(&left, right.clone());
    assert_eq!(merged, Prompt::make([left.messages, right].concat()));
}

#[test]
fn returns_empty_when_both_inputs_have_no_messages() {
    // Prompt.test.ts:499
    assert_eq!(Prompt::concat(&Prompt::empty(), vec![]), Prompt::empty());
}

#[test]
fn accepts_raw_message_input_when_the_left_prompt_is_empty() {
    // Prompt.test.ts:504 — a bare string becomes one text part.
    let merged = Prompt::concat(
        &Prompt::empty(),
        vec![
            PromptMessage::text(Role::User, "a"),
            PromptMessage::text(Role::Assistant, "b"),
        ],
    );
    assert_eq!(
        merged,
        Prompt::make(vec![
            PromptMessage::text(Role::User, "a"),
            PromptMessage::text(Role::Assistant, "b"),
        ])
    );
}

#[test]
fn returns_the_original_prompt_when_the_input_has_no_messages() {
    // Prompt.test.ts:523
    let prompt = Prompt::make(vec![
        PromptMessage::text(Role::User, "a"),
        PromptMessage::text(Role::Assistant, "b"),
    ]);
    assert_eq!(Prompt::concat(&prompt, vec![]), prompt);
}

#[test]
fn appends_text_to_the_leading_system_message() {
    // Prompt.test.ts:539
    let prompt = Prompt::make(vec![
        PromptMessage::text(Role::System, "You are an expert in programming."),
        PromptMessage::text(Role::User, "Hello, world!"),
    ]);
    let result = Prompt::append_system(&prompt, " You are a helpful assistant.");
    assert_eq!(
        result.messages[0],
        PromptMessage::text(
            Role::System,
            "You are an expert in programming. You are a helpful assistant."
        )
    );
}

#[test]
fn creates_a_leading_system_message_when_appending_with_none_present() {
    // Prompt.test.ts:555
    let prompt = Prompt::make(vec![PromptMessage::text(Role::User, "Hello, world!")]);
    let result = Prompt::append_system(&prompt, "You are a helpful assistant.");
    assert_eq!(
        result.messages[0],
        PromptMessage::text(Role::System, "You are a helpful assistant.")
    );
}

#[test]
fn creates_a_system_message_when_appending_to_an_empty_prompt() {
    // Prompt.test.ts:570
    let result = Prompt::append_system(&Prompt::empty(), "You are a helpful assistant.");
    assert_eq!(
        result.messages[0],
        PromptMessage::text(Role::System, "You are a helpful assistant.")
    );
}

#[test]
fn prepends_text_to_the_leading_system_message() {
    // Prompt.test.ts:583
    let prompt = Prompt::make(vec![
        PromptMessage::text(Role::System, "You are an expert in programming."),
        PromptMessage::text(Role::User, "Hello, world!"),
    ]);
    let result = Prompt::prepend_system(&prompt, "You are a helpful assistant. ");
    assert_eq!(
        result.messages[0],
        PromptMessage::text(
            Role::System,
            "You are a helpful assistant. You are an expert in programming."
        )
    );
}

#[test]
fn creates_a_leading_system_message_when_prepending_with_none_present() {
    // Prompt.test.ts:605
    let prompt = Prompt::make(vec![PromptMessage::text(Role::User, "Hello, world!")]);
    let result = Prompt::prepend_system(&prompt, "You are a helpful assistant.");
    assert_eq!(
        result.messages[0],
        PromptMessage::text(Role::System, "You are a helpful assistant.")
    );
}

#[test]
fn creates_a_system_message_when_prepending_to_an_empty_prompt() {
    // Prompt.test.ts:623
    let result = Prompt::prepend_system(&Prompt::empty(), "You are a helpful assistant.");
    assert_eq!(
        result.messages[0],
        PromptMessage::text(Role::System, "You are a helpful assistant.")
    );
}

fn prompt_as_json(prompt: &Prompt) -> Value {
    serde_json::to_value(
        prompt
            .messages
            .iter()
            .map(|message| {
                serde_json::json!({
                    "role": format!("{:?}", message.role).to_lowercase(),
                    "parts": message.parts.iter().map(part_as_json).collect::<Vec<_>>(),
                })
            })
            .collect::<Vec<_>>(),
    )
    .expect("encodes")
}

fn part_as_json(part: &PromptPart) -> Value {
    match part {
        PromptPart::Text { text, options } => {
            json_with_options("text", serde_json::json!({"text": text}), options)
        }
        PromptPart::Reasoning { text, options } => {
            json_with_options("reasoning", serde_json::json!({"text": text}), options)
        }
        PromptPart::File {
            media_type,
            data_base64,
            options,
        } => json_with_options(
            "file",
            serde_json::json!({"mediaType": media_type, "data": data_base64}),
            options,
        ),
        PromptPart::ToolCall {
            id,
            name,
            params,
            provider_executed,
            options,
        } => json_with_options(
            "tool-call",
            serde_json::json!({"id": id, "name": name, "params": params, "providerExecuted": provider_executed}),
            options,
        ),
        PromptPart::ToolResult {
            id,
            name,
            is_failure,
            result,
            provider_executed,
            preliminary,
            options,
        } => json_with_options(
            "tool-result",
            serde_json::json!({"id": id, "name": name, "isFailure": is_failure, "result": result, "providerExecuted": provider_executed, "preliminary": preliminary}),
            options,
        ),
        PromptPart::ToolApprovalRequest {
            approval_id,
            tool_call_id,
            options,
        } => json_with_options(
            "tool-approval-request",
            serde_json::json!({"approvalId": approval_id, "toolCallId": tool_call_id}),
            options,
        ),
        PromptPart::ToolApprovalResponse {
            approval_id,
            approved,
            options,
        } => json_with_options(
            "tool-approval-response",
            serde_json::json!({"approvalId": approval_id, "approved": approved}),
            options,
        ),
    }
}

fn json_with_options(kind: &str, mut body: Value, options: &Map<String, Value>) -> Value {
    let map = body.as_object_mut().expect("object body");
    map.insert("type".to_owned(), Value::String(kind.to_owned()));
    for (key, value) in options {
        map.insert(format!("options:{key}"), value.clone());
    }
    body
}

fn prompt_from_json(value: &Value) -> Option<Prompt> {
    let messages = value.as_array()?;
    let mut out = Vec::with_capacity(messages.len());
    for message in messages {
        let role = match message.get("role")?.as_str()? {
            "system" => Role::System,
            "user" => Role::User,
            "assistant" => Role::Assistant,
            "tool" => Role::Tool,
            _ => return None,
        };
        let mut parts = Vec::new();
        for part in message.get("parts")?.as_array()? {
            parts.push(part_from_json(part)?);
        }
        out.push(PromptMessage::new(role, parts));
    }
    Some(Prompt { messages: out })
}

fn part_from_json(value: &Value) -> Option<PromptPart> {
    let get_options = || {
        value
            .as_object()
            .map(|map| {
                map.iter()
                    .filter_map(|(key, item)| {
                        key.strip_prefix("options:")
                            .map(|name| (name.to_owned(), item.clone()))
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    Some(match value.get("type")?.as_str()? {
        "text" => PromptPart::Text {
            text: value.get("text")?.as_str()?.to_owned(),
            options: get_options(),
        },
        "reasoning" => PromptPart::Reasoning {
            text: value.get("text")?.as_str()?.to_owned(),
            options: get_options(),
        },
        "file" => PromptPart::File {
            media_type: value.get("mediaType")?.as_str()?.to_owned(),
            data_base64: value.get("data")?.as_str()?.to_owned(),
            options: get_options(),
        },
        "tool-call" => PromptPart::ToolCall {
            id: value.get("id")?.as_str()?.to_owned(),
            name: value.get("name")?.as_str()?.to_owned(),
            params: value.get("params")?.clone(),
            provider_executed: value.get("providerExecuted")?.as_bool()?,
            options: get_options(),
        },
        "tool-result" => PromptPart::ToolResult {
            id: value.get("id")?.as_str()?.to_owned(),
            name: value.get("name")?.as_str()?.to_owned(),
            is_failure: value.get("isFailure")?.as_bool()?,
            result: value.get("result")?.clone(),
            provider_executed: value.get("providerExecuted")?.as_bool()?,
            preliminary: value.get("preliminary")?.as_bool().unwrap_or(false),
            options: get_options(),
        },
        "tool-approval-request" => PromptPart::ToolApprovalRequest {
            approval_id: value.get("approvalId")?.as_str()?.to_owned(),
            tool_call_id: value.get("toolCallId")?.as_str()?.to_owned(),
            options: get_options(),
        },
        "tool-approval-response" => PromptPart::ToolApprovalResponse {
            approval_id: value.get("approvalId")?.as_str()?.to_owned(),
            approved: value.get("approved")?.as_bool()?,
            options: get_options(),
        },
        _ => return None,
    })
}
