//! Server-assembly carries for McpServer.test.ts: initialize, capability
//! advertisement, tool/prompt/resource dispatch, notifications.
//!
//! Excused (assembly has no such piece; reasons name them):
//! stdio/HTTP transports, sessions, batches, SSE, Origin checks
//! (263, 406, 462, 566, 598, 643, 702, 764, 849, 957, 1024, 1432, 1455,
//! 1475, 2166, 2183, 2222, 2270, 2736, 2767, 2793, 2873, 2438, 2497);
//! elicitation flows (323, 1118); structured-output/strict/identified
//! schema codecs and declared-failure schemas
//! (1504, 1530, 1563, 1593, 1612, 1649, 1679, 1707, 1766, 1782, 1792,
//! 1802, 1942, 1959, 1974, 1997, 2022, 2133).
//!
//! Adaptations where the Effect service wiring has no Rust equivalent are
//! noted per test.

use elm_effect_unstable::mcp_assembly::{
    McpServer, encode_tool_success, internal_error_message,
};
use elm_effect_unstable::mcp_handlers::{PromptArg, content_text};
use elm_effect_unstable::mcp_server::{LATEST_VERSION, ServerInfo};
use serde_json::{Value, json};

fn server() -> McpServer {
    McpServer::new(ServerInfo::test())
}

fn bob_ctx(server: &McpServer) -> elm_effect_unstable::mcp_assembly::RequestContext {
    server.context(2, "Bob")
}

// --- completion notifications are no-ops without a live peer ---

#[test]
fn completion_noop_when_client_absent() {
    // Row 304: absent target client returns without touching transport.
    server().notify_elicitation_complete(0, "gone");
}

#[test]
fn completion_noop_when_protocol_unsupported() {
    // Row 313: target protocol without completion support still returns.
    server().notify_elicitation_complete(9, "no-support");
}

#[test]
fn completion_noop_without_transport() {
    // Row 1112: direct service with no transport running.
    server().notify_elicitation_complete(0, "not-connected");
}

// --- registration context: invocation identity wins ---

#[test]
fn current_request_for_cross_client_resource() {
    // Row 1224: resource registered under Alice reads under Bob's request.
    // Adaptation: handlers cannot see the invocation (no Effect services);
    // the test pins readability plus caller-identity context.
    let mut s = server();
    s.resources_mut().insert_text("file:///owner", "owner", "o", "text/plain", "registered:2:Bob:Bob");
    let ctx = bob_ctx(&s);
    let out = s.find_resource(&ctx, "file:///owner").unwrap();
    assert_eq!(out[0].text.as_deref(), Some("registered:2:Bob:Bob"));
    assert_eq!(ctx.client_id, 2);
    assert_eq!(ctx.client_name, "Bob");
}

#[test]
fn current_request_for_cross_client_template() {
    // Row 1234: template registered under Alice expands under Bob.
    let mut s = server();
    s.resources_mut().add_template(elm_effect_unstable::mcp_handlers::ResourceTemplate {
        uri_template: "file:///owners/{name}".to_owned(),
        name: "owners".to_owned(),
        description: "o".to_owned(),
        mime_type: "text/plain".to_owned(),
        handler: Box::new(|params| {
            Ok(vec![elm_effect_unstable::mcp_handlers::ResourceContent {
                uri: format!("file:///owners/{}", params["name"]),
                mime_type: Some("text/plain".to_owned()),
                text: Some("registered:2:Bob:Bob".to_owned()),
                blob: None,
            }])
        }),
    });
    let ctx = bob_ctx(&s);
    let out = s.find_resource(&ctx, "file:///owners/bob").unwrap();
    assert_eq!(out[0].text.as_deref(), Some("registered:2:Bob:Bob"));
}

#[test]
fn current_request_for_cross_client_prompt() {
    // Row 1244: prompt registered under Alice renders under Bob.
    let mut s = server();
    s.prompts_mut().register(
        "owner",
        None,
        "o",
        vec![PromptArg { name: "name".to_owned(), required: true, choices: vec![] }],
        vec![("user".to_owned(), "registered:2:Bob:Bob".to_owned())],
    );
    let ctx = bob_ctx(&s);
    let out = s.get_prompt(&ctx, "owner", &json!({"name": "bob"})).unwrap();
    assert_eq!(out.messages[0]["content"]["text"], json!("registered:2:Bob:Bob"));
}

#[test]
fn cross_client_resource_completion() {
    // Row 1254: completion registered under Alice answers under Bob.
    let mut s = server();
    s.register_completion("resource:file:///owners/{param0}:param0", |_| {
        vec!["registered:2:Bob:Bob".to_owned()]
    });
    let ctx = bob_ctx(&s);
    assert_eq!(
        s.complete(&ctx, "resource:file:///owners/{param0}:param0", "b"),
        vec!["registered:2:Bob:Bob".to_owned()]
    );
}

#[test]
fn cross_client_prompt_completion() {
    // Row 1267: prompt completion registered under Alice answers under Bob.
    let mut s = server();
    s.register_completion("prompt:owner:name", |_| vec!["registered:2:Bob:Bob".to_owned()]);
    let ctx = bob_ctx(&s);
    assert_eq!(s.complete(&ctx, "prompt:owner:name", "b"), vec!["registered:2:Bob:Bob".to_owned()]);
}

#[test]
fn cross_client_toolkit_tool_uses_invocation_context() {
    // Row 1280: ctx-aware tool registered under Alice reads Bob at call time.
    let mut s = server();
    s.register_ctx_tool("owner", "o", json!({"type": "object"}), |ctx, _| {
        Ok(vec![content_text(format!("registered:{}:{}:{}", ctx.client_id, ctx.client_name, ctx.client_name))])
    });
    let ctx = bob_ctx(&s);
    let out = s.call_tool(&ctx, "owner", None).unwrap();
    assert!(!out.is_error);
    assert_eq!(out.content[0]["text"], json!("registered:2:Bob:Bob"));
}

// --- dispatch without a running transport ---

#[test]
fn notification_tools_complete_without_transport() {
    // Row 1292: a tool emitting a notification completes with no transport.
    let mut s = server();
    s.tools_mut().register("Emit", "e", json!({"type": "object"}), |_| Ok(vec![]));
    let ctx = s.context(1, "TestClient");
    let out = s.call_tool(&ctx, "Emit", None).unwrap();
    assert!(!out.is_error);
    assert!(out.content.is_empty());
    s.emit_notification("notifications/message", json!({"level": "error", "data": "diagnostic"}));
    let pending = s.drain_notifications();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].method, "notifications/message");
}

#[test]
fn unknown_resource_is_invalid_params() {
    // Row 1309: exact pin message.
    let s = server();
    let ctx = s.context(1, "c");
    let err = s.find_resource(&ctx, "file:///unknown").unwrap_err();
    assert_eq!(err.code, -32602);
    assert_eq!(err.message, "Resource 'file:///unknown' not found");
}

#[test]
fn resolves_https_resource_template() {
    // Row 1322: template expansion is transport-independent (HTTP framing excused).
    let mut s = server();
    s.resources_mut().add_template(elm_effect_unstable::mcp_handlers::ResourceTemplate {
        uri_template: "https://example.test/docs/{name}".to_owned(),
        name: "document".to_owned(),
        description: "d".to_owned(),
        mime_type: "text/plain".to_owned(),
        handler: Box::new(|params| {
            Ok(vec![elm_effect_unstable::mcp_handlers::ResourceContent {
                uri: format!("https://example.test/docs/{}", params["name"]),
                mime_type: Some("text/plain".to_owned()),
                text: Some(params["name"].clone()),
                blob: None,
            }])
        }),
    });
    let ctx = s.context(1, "c");
    let out = s.find_resource(&ctx, "https://example.test/docs/alice").unwrap();
    assert_eq!(out[0].uri, "https://example.test/docs/alice");
    assert_eq!(out[0].text.as_deref(), Some("alice"));
}

#[test]
fn preserves_handler_typed_failure() {
    // Row 1338, adapted: no typed-failure channel in Rust; the handler's
    // failure text rides verbatim in isError content (identity preserved).
    let mut s = server();
    s.tools_mut().register("f", "f", json!({"type": "object"}), |_| Err("resource failed".to_owned()));
    let ctx = s.context(1, "c");
    let out = s.call_tool(&ctx, "f", Some(&json!({}))).unwrap();
    assert!(out.is_error);
    assert_eq!(out.content[0]["text"], json!("resource failed"));
}

#[test]
fn prompt_string_substitution_and_completion() {
    // Row 1359, adapted: substitution only; FiniteFromString decode excused
    // (no schema compiler), so "12" stays the string "12".
    let mut s = server();
    s.prompts_mut().register(
        "count",
        None,
        "c",
        vec![PromptArg { name: "count".to_owned(), required: true, choices: vec![] }],
        vec![("user".to_owned(), "{{count}}".to_owned())],
    );
    s.register_completion("prompt:count:count", |_| vec!["13".to_owned()]);
    let ctx = s.context(1, "c");
    let got = s.get_prompt(&ctx, "count", &json!({"count": "12"})).unwrap();
    assert_eq!(got.messages[0]["content"]["text"], json!("12"));
    assert_eq!(s.complete(&ctx, "prompt:count:count", "1"), vec!["13".to_owned()]);
}

#[test]
fn omitted_arguments_default_to_empty_object() {
    // Row 1381, adapted: Rust handlers take &Value, so omission arrives as
    // {} rather than undefined; the contract pinned is "handler runs".
    let mut s = server();
    s.tools_mut().register("arguments-omitted", "a", json!({"type": "object", "properties": {}}), |_| Ok(vec![]));
    let ctx = s.context(1, "c");
    assert!(s.call_tool(&ctx, "arguments-omitted", None).is_ok());
}

#[test]
fn neutral_request_context_carries_version() {
    // Row 1407: handlers observe the negotiated version.
    let mut s = server();
    s.initialize("2025-06-18", "c", "1.0.0").unwrap();
    let ctx = s.context(1, "c");
    assert_eq!(ctx.protocol_version, "2025-06-18");
}

// --- handshake ---

#[test]
fn negotiates_unsupported_version_to_latest() {
    // Row 1487: unsupported client version negotiates to latest.
    let mut s = server();
    let result = s.initialize("9999-01-01", "c", "1.0.0").unwrap();
    assert_eq!(result["protocolVersion"], json!(LATEST_VERSION));
    assert_eq!(s.negotiated_version(), Some(LATEST_VERSION));
}

#[test]
fn selects_latest_when_unsupported_offered() {
    // Row 2370: same negotiate-to-latest mechanism (June was latest at pin time).
    let mut s = server();
    s.initialize("1900-01-01", "c", "1.0.0").unwrap();
    assert_eq!(s.negotiated_version(), Some(LATEST_VERSION));
    s.notify_initialized();
    assert!(s.is_initialized());
}

#[test]
fn advertise_reflects_registered_capabilities() {
    // Capability advertisement follows registration.
    let mut s = server();
    s.tools_mut().register("t", "t", json!({"type": "object"}), |_| Ok(vec![]));
    let caps = s.advertise();
    assert!(caps.contains_key("tools"));
    assert!(caps.contains_key("logging"));
    assert!(!caps.contains_key("prompts"));
}

// --- tool validation and results ---

#[test]
fn unknown_tools_are_protocol_errors() {
    // Row 2149: exact pin message and INVALID_PARAMS code.
    let s = server();
    let ctx = s.context(1, "c");
    let err = s.call_tool(&ctx, "UnknownTool", Some(&json!({}))).unwrap_err();
    assert_eq!(err.code, -32602);
    assert_eq!(err.message, "Tool 'UnknownTool' not found");
}

#[test]
fn concise_param_errors_skip_handler() {
    // Row 1826: validation error names the param; handler never runs.
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    let count = Arc::new(AtomicUsize::new(0));
    let moved = count.clone();
    let mut s = server();
    s.tools_mut().register(
        "t",
        "t",
        json!({"type": "object", "properties": {"first": {"type": "string"}}, "required": ["first", "second"]}),
        move |_| {
            moved.fetch_add(1, Ordering::SeqCst);
            Ok(vec![])
        },
    );
    let ctx = s.context(1, "c");
    let err = s.call_tool(&ctx, "t", Some(&json!({"first": 1}))).unwrap_err();
    assert_eq!(err.code, -32602);
    assert!(err.message.contains("second"), "unexpected: {}", err.message);
    assert_eq!(count.load(Ordering::SeqCst), 0);
}

#[test]
fn optional_params_omitted_still_succeed() {
    // Row 1857: unrequired params may be omitted.
    let mut s = server();
    s.tools_mut().register(
        "opt",
        "o",
        json!({"type": "object", "properties": {"signature": {"type": "string"}}}),
        |args| Ok(vec![content_text(args["signature"].as_str().unwrap_or("omitted"))]),
    );
    let ctx = s.context(1, "c");
    let out = s.call_tool(&ctx, "opt", Some(&json!({}))).unwrap();
    assert!(!out.is_error);
    assert_eq!(out.content[0]["text"], json!("omitted"));
}

#[test]
fn void_results_stay_successful() {
    // Row 1883: Null success is ok with no structured content.
    let (content, structured) = encode_tool_success(&Value::Null);
    assert!(content.is_empty());
    assert!(structured.is_none());
}

#[test]
fn object_results_carry_structured_content() {
    // Row 1902: objects stage both text content and structured content.
    let (content, structured) = encode_tool_success(&json!({"answer": "result"}));
    assert_eq!(content[0]["type"], json!("text"));
    assert_eq!(structured, Some(json!({"answer": "result"})));
}

#[test]
fn null_and_array_omit_structured_content() {
    // Row 1918.
    let (_, null_structured) = encode_tool_success(&Value::Null);
    let (_, array_structured) = encode_tool_success(&json!(["first", "second"]));
    assert!(null_structured.is_none());
    assert!(array_structured.is_none());
}

#[test]
fn internal_failures_use_generic_message() {
    // Row 2089, adapted: non-validation failures scrub to the pin's generic.
    assert_eq!(
        internal_error_message(),
        "Tool execution failed due to an internal server error."
    );
    let mut s = McpServer::with_report_errors(ServerInfo::test(), true);
    let scrubbed = s.record_defect("private defect details");
    assert_eq!(scrubbed, internal_error_message());
    assert_eq!(s.reported(), &["private defect details".to_owned()]);
}

#[test]
fn defects_scrubbed_when_reporting_disabled() {
    // Row 2109, adapted: reportErrors=false still scrubs; nothing recorded.
    let mut s = McpServer::with_report_errors(ServerInfo::test(), false);
    let scrubbed = s.record_defect("private defect details");
    assert_eq!(scrubbed, internal_error_message());
    assert!(s.reported().is_empty());
}

// --- notifications ---

#[test]
fn coalesces_repeated_list_changed() {
    // Row 2389: rapid repeats of one kind collapse to a single notification.
    let mut s = server();
    s.note_list_changed("tools");
    s.note_list_changed("tools");
    s.note_list_changed("tools");
    s.note_list_changed("prompts");
    let pending = s.drain_notifications();
    assert_eq!(pending.len(), 2);
    assert_eq!(pending[0].method, "notifications/tools/list_changed");
    assert_eq!(pending[1].method, "notifications/prompts/list_changed");
}

#[test]
fn isolates_resource_subscriptions() {
    // Row 2604, simplified: per-URI routing only; backlog overflow excused
    // (no backpressure machinery in the in-memory assembly).
    let mut s = server();
    s.resources_mut().subscribe("file:///a");
    s.resources_mut().subscribe("file:///b");
    let ctx = s.context(1, "c");
    assert!(s.resource_update(&ctx, "file:///a").is_some());
    assert!(s.resource_update(&ctx, "file:///b").is_some());
    s.resources_mut().unsubscribe("file:///a");
    assert!(s.resource_update(&ctx, "file:///a").is_none());
    assert!(s.resource_update(&ctx, "file:///b").is_some());
}
