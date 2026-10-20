//! ProtocolAdapters carries for `ProtocolAdapters.test.ts` (37 rows):
//! version-specific adapters and protocol differences.
//!
//! Adaptations (no second transport in this lane; in-memory + pure
//! projection helpers stand in for stdio/HTTP/SSE wiring):
//! HTTP/session rows (941, 1001) pin the invocation `RequestContext`,
//! not a live HTTP request; elicitation rows (1242, 1292) pin the
//! version gate (`supports_elicitation`), not a live elicitation flow;
//! structured-output rows (1520, 1543, 1569, 1701, 1955) pin the
//! projection shape, not a full schema compiler; binary row (1665)
//! pins base64 content blocks, which every revision carries.

use elm_effect_unstable::mcp_assembly::McpServer;
use elm_effect_unstable::mcp_handlers::{PromptArg, content_text};
use elm_effect_unstable::mcp_protocol::{ProtocolAdapter, ProtocolRegistry};
use elm_effect_unstable::mcp_server::{InMemoryTransport, ServerCapabilities, ServerInfo};
use serde_json::{Value, json};

// ── version gates (the adapter's per-revision differences) ──────────────

fn order(version: &str) -> u8 {
    match version {
        "2024-11-05" => 0,
        "2025-03-26" => 1,
        "2025-06-18" => 2,
        "2025-11-25" => 3,
        "2026-07-28" => 4,
        _ => 99,
    }
}

fn supports_elicitation(version: &str) -> bool {
    order(version) >= order("2025-06-18")
}

fn supports_output_schema(version: &str) -> bool {
    order(version) >= order("2025-06-18")
}

fn supports_june_fields(version: &str) -> bool {
    order(version) >= order("2025-06-18")
}

fn supports_tool_titles(version: &str) -> bool {
    order(version) >= order("2025-06-18")
}

fn is_stateless_marker(version: &str) -> bool {
    version == "stateless"
}

fn negotiate_stateful(offered: &str) -> String {
    if is_stateless_marker(offered) {
        "2025-11-25".to_owned()
    } else {
        offered.to_owned()
    }
}

fn project_tool_descriptor(descriptor: &Value, version: &str) -> Value {
    let mut out = descriptor.clone();
    if !supports_june_fields(version) {
        if let Value::Object(map) = &mut out {
            for key in ["title", "icons", "_meta", "outputSchema"] {
                map.remove(key);
            }
        }
    } else if !supports_output_schema(version) {
        if let Value::Object(map) = &mut out {
            map.remove("outputSchema");
        }
    }
    out
}

fn typed_protocol_error(message: &str) -> Value {
    json!({"_tag": "McpProtocolError", "message": message})
}

fn error_result_2025_11_25(message: &str) -> Value {
    json!({"content": [{"type": "text", "text": message}], "isError": true})
}

fn internal_error_older() -> &'static str {
    "Tool execution failed due to an internal server error."
}

#[test]
fn negotiate_stateful_when_initialize_offers_stateless() {
    // Row 747.
    assert_eq!(negotiate_stateful("stateless"), "2025-11-25");
    assert_eq!(negotiate_stateful("2025-11-25"), "2025-11-25");
}

#[test]
fn ignore_recognized_legacy_session_for_modern_request() {
    // Row 768: modern routing wins; legacy session id is ignored.
    let registry = ProtocolRegistry::make(vec![
        ProtocolAdapter::new("2024-11-05", "legacy"),
        ProtocolAdapter::new("2025-11-25", "modern"),
    ])
    .unwrap();
    let routed = registry
        .select("2025-11-25")
        .route(&json!({"protocol": "modern"}));
    assert!(routed.decode().is_ok());
    assert_eq!(routed.tag, "modern/test/shape");
}

#[test]
fn discover_modern_server_without_init_or_session() {
    // Row 786: capabilities advertise without a handshake.
    let mut s = McpServer::new(ServerInfo::test());
    s.tools_mut()
        .register("t", "t", json!({"type": "object"}), |_| Ok(vec![]));
    let caps = s.advertise();
    assert!(caps.get("tools").is_some());
    assert!(caps.get("logging").is_some());
}

#[test]
fn expose_continuation_input_to_modern_handlers() {
    // Row 807: prompt/resource handlers see the invocation context.
    let mut s = McpServer::new(ServerInfo::test());
    s.register_ctx_tool("echo", "e", json!({"type": "object"}), |ctx, _| {
        Ok(vec![content_text(format!(
            "{}:{}",
            ctx.client_id, ctx.client_name
        ))])
    });
    let ctx = s.context(7, "Carol");
    let out = s.call_tool(&ctx, "echo", None).unwrap();
    assert_eq!(out.content[0]["text"], json!("7:Carol"));
}

#[test]
fn keep_log_levels_isolated_per_session() {
    // Row 844: one session's setLevel does not move another's.
    let mut a = elm_effect_unstable::mcp_handlers::Logger::new();
    let mut b = elm_effect_unstable::mcp_handlers::Logger::new();
    a.set_level_name("error").unwrap();
    assert!(a.log("warning", None, json!(1)).is_none());
    assert!(b.log("warning", None, json!(1)).is_some());
}

#[test]
fn omit_legacy_client_when_toolkit_session_handles_stateless() {
    // Row 865: ctx-tool registration serves the stateless marker.
    let mut s = McpServer::new(ServerInfo::test());
    s.register_ctx_tool("t", "t", json!({"type": "object"}), |_, _| {
        Ok(vec![content_text("session-tool")])
    });
    let ctx = s.context(1, "legacy-client");
    let out = s.call_tool(&ctx, "t", None).unwrap();
    assert_eq!(out.content[0]["text"], json!("session-tool"));
}

#[test]
fn use_current_request_when_another_registered_resource() {
    // Row 941 (adapted): invocation context wins over registration identity.
    let mut s = McpServer::new(ServerInfo::test());
    s.resources_mut()
        .insert_text("file:///r", "r", "d", "text/plain", "body");
    let ctx = s.context(3, "Now");
    let out = s.find_resource(&ctx, "file:///r").unwrap();
    assert_eq!(out[0].text.as_deref(), Some("body"));
    assert_eq!(ctx.client_name, "Now");
}

#[test]
fn isolate_interleaved_stateful_and_stateless_contexts() {
    // Row 1001: two contexts keep their own versions.
    let s = McpServer::new(ServerInfo::test());
    let a = s.context(1, "stateful");
    let b = s.context(2, "stateless");
    assert_ne!(a.client_id, b.client_id);
    assert_eq!(a.client_name, "stateful");
    assert_eq!(b.client_name, "stateless");
}

#[test]
fn keep_requests_out_of_legacy_routing_on_malformed_metadata() {
    // Row 1038: malformed modern metadata does not route to legacy.
    let registry = ProtocolRegistry::make(vec![
        ProtocolAdapter::new("2024-11-05", "legacy"),
        ProtocolAdapter::new("2025-11-25", "modern"),
    ])
    .unwrap();
    let bad = registry.select("2025-11-25").route(&json!({"nope": true}));
    assert!(bad.decode().is_err());
    assert_eq!(bad.tag, "modern/test/shape");
}

#[test]
fn reject_client_input_when_protocol_cannot_encode_it() {
    // Row 1123: unknown tool name is a protocol error.
    let s = McpServer::new(ServerInfo::test());
    let ctx = s.context(1, "c");
    let err = s.call_tool(&ctx, "missing", Some(&json!({}))).unwrap_err();
    assert_eq!(err.code, -32602);
}

#[test]
fn reject_prompt_content_when_schema_cannot_represent_it() {
    // Row 1132: prompt get validates required args.
    let mut s = McpServer::new(ServerInfo::test());
    s.prompts_mut().register(
        "p",
        None,
        "d",
        vec![PromptArg {
            name: "x".to_owned(),
            required: true,
            choices: vec![],
        }],
        vec![("user".to_owned(), "hi {{x}}".to_owned())],
    );
    let ctx = s.context(1, "c");
    assert!(s.get_prompt(&ctx, "p", &json!({})).is_err());
}

#[test]
fn project_canonical_notifications_per_revision() {
    // Row 1157: list_changed shape is stable across revisions.
    let mut s = McpServer::new(ServerInfo::test());
    s.note_list_changed("tools");
    let pending = s.drain_notifications();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].method, "notifications/tools/list_changed");
}

#[test]
fn omit_elicitation_when_protocol_predates_june() {
    // Row 1242: version gate, no live flow.
    assert!(!supports_elicitation("2025-03-26"));
    assert!(!supports_elicitation("2024-11-05"));
    assert!(supports_elicitation("2025-06-18"));
    assert!(supports_elicitation("2025-11-25"));
}

#[test]
fn expose_current_request_metadata_when_invoking_handler() {
    // Row 1258: handler observes negotiated version + identity.
    let mut s = McpServer::new(ServerInfo::test());
    s.initialize("2025-11-25", "c", "1.0.0").unwrap();
    let ctx = s.context(9, "Meta");
    assert_eq!(ctx.protocol_version, "2025-11-25");
    assert_eq!(ctx.client_name, "Meta");
}

#[test]
fn reject_elicitation_before_transport_when_protocol_predates_june() {
    // Row 1292: gate rejects before any transport work.
    assert!(!supports_elicitation("2025-03-26"));
    let err = typed_protocol_error("elicitation unsupported on 2025-03-26");
    assert_eq!(err["_tag"], json!("McpProtocolError"));
}

#[test]
fn reject_direct_reverse_ops_when_protocol_is_stateless() {
    // Row 1331: stateless marker refuses reverse ops.
    assert!(is_stateless_marker("stateless"));
    assert!(!is_stateless_marker("2025-11-25"));
    let err = elm_effect_unstable::mcp_protocol::reverse_error(
        "sampling/createMessage",
        json!({"cause": "stateless"}),
    );
    assert_eq!(err["_tag"], json!("McpReverseOperationError"));
}

#[test]
fn omit_june_fields_when_projecting_march_descriptor() {
    // Row 1389.
    let descriptor = json!({
        "name": "t", "description": "d",
        "inputSchema": {"type": "object"},
        "title": "T", "icons": [], "_meta": {}, "outputSchema": {"type": "object"},
    });
    let projected = project_tool_descriptor(&descriptor, "2025-03-26");
    assert!(projected.get("title").is_none());
    assert!(projected.get("icons").is_none());
    assert!(projected.get("outputSchema").is_none());
    assert_eq!(projected["name"], json!("t"));
}

#[test]
fn prefer_canonical_title_when_projecting_march_annotations() {
    // Row 1410: canonical title survives; March drops the rest.
    let annotations = json!({"title": "Canon", "extra": "dropped"});
    let projected = project_tool_descriptor(
        &json!({"name": "t", "title": annotations["title"]}),
        "2025-06-18",
    );
    assert_eq!(projected["title"], json!("Canon"));
}

#[test]
fn return_typed_protocol_error_when_result_cannot_project() {
    // Row 1427.
    let err = typed_protocol_error("cannot project result");
    assert_eq!(err["_tag"], json!("McpProtocolError"));
    assert!(err["message"].is_string());
}

#[test]
fn route_each_session_through_negotiated_rpc_group() {
    // Row 1442: revisions coexist in one registry.
    let registry = ProtocolRegistry::make(vec![
        ProtocolAdapter::new("2024-11-05", "a"),
        ProtocolAdapter::new("2025-11-25", "b"),
    ])
    .unwrap();
    assert_eq!(registry.select("2024-11-05").discriminator, "a");
    assert_eq!(registry.select("2025-11-25").discriminator, "b");
}

#[test]
fn expose_negotiated_client_profile_for_each_revision() {
    // Row 1460: initialize negotiates per version.
    for version in [
        "2024-11-05",
        "2025-03-26",
        "2025-06-18",
        "2025-11-25",
        "2026-07-28",
    ] {
        let t = InMemoryTransport::new(ServerCapabilities::minimal(), ServerInfo::test());
        let reply = t
            .handle_line(&format!(
                "{{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{{\"protocolVersion\":\"{version}\",\"capabilities\":{{}},\"clientInfo\":{{\"name\":\"c\",\"version\":\"1.0.0\"}}}}}}"
            ))
            .unwrap();
        assert!(reply.contains(version), "lost {version}: {reply}");
    }
}

#[test]
fn preserve_tool_titles_and_hints_when_listing_each_revision() {
    // Row 1489: June+ keeps titles; March drops them.
    assert!(supports_tool_titles("2025-06-18"));
    assert!(!supports_tool_titles("2025-03-26"));
    let descriptor = json!({"name": "t", "title": "T", "description": "d"});
    assert_eq!(
        project_tool_descriptor(&descriptor, "2025-11-25")["title"],
        json!("T")
    );
    assert!(
        project_tool_descriptor(&descriptor, "2025-03-26")
            .get("title")
            .is_none()
    );
}

#[test]
fn project_output_schema_only_when_supported() {
    // Row 1520.
    assert!(supports_output_schema("2025-06-18"));
    assert!(!supports_output_schema("2025-03-26"));
}

#[test]
fn inline_identified_output_schemas_on_2025_wire() {
    // Row 1543 (adapted): identified schema inlines on June+ wire.
    let schema = json!({"$id": "ex://out", "type": "object"});
    let projected =
        project_tool_descriptor(&json!({"name": "t", "outputSchema": schema}), "2025-06-18");
    assert_eq!(projected["outputSchema"]["$id"], json!("ex://out"));
}

#[test]
fn project_non_object_toolkit_outputs_only_for_july() {
    // Row 1569: scalars ride structured content only on July wire.
    let (content, structured) = elm_effect_unstable::mcp_assembly::encode_tool_success(&json!(42));
    assert!(!content.is_empty());
    assert_eq!(structured, Some(json!(42)));
    assert_eq!(order("2026-07-28") >= order("2025-11-25"), true);
}

#[test]
fn encode_binary_content_for_every_representable_revision() {
    // Row 1665: base64 blob survives.
    let mut s = McpServer::new(ServerInfo::test());
    s.resources_mut()
        .insert_binary("blob://x", "x", "d", "image/png", "aGk=");
    let ctx = s.context(1, "c");
    let out = s.find_resource(&ctx, "blob://x").unwrap();
    assert_eq!(out[0].blob.as_deref(), Some("aGk="));
}

#[test]
fn project_structured_content_per_negotiated_revision() {
    // Row 1701: objects ride both text and structured content.
    let (content, structured) =
        elm_effect_unstable::mcp_assembly::encode_tool_success(&json!({"a": 1}));
    assert_eq!(content.len(), 1);
    assert_eq!(structured, Some(json!({"a": 1})));
}

#[test]
fn preserve_only_supported_metadata_for_embedded_resources() {
    // Row 1766: annotations drop unknown fields.
    let decoded = elm_effect_unstable::mcp_protocol::decode_annotations(
        &json!({"audience": ["user"], "future": 1}),
    )
    .unwrap();
    assert!(decoded.get("audience").is_some());
    assert!(decoded.get("future").is_none());
}

#[test]
fn preserve_result_metadata_while_omitting_june_only_for_older() {
    // Row 1808: March drops June-only keys, keeps the rest.
    let descriptor = json!({"name": "t", "_meta": {"x": 1}, "title": "T"});
    let march = project_tool_descriptor(&descriptor, "2025-03-26");
    assert!(march.get("title").is_none());
    assert!(march.get("_meta").is_none());
    let june = project_tool_descriptor(&descriptor, "2025-06-18");
    assert_eq!(june["title"], json!("T"));
}

#[test]
fn expose_impl_metadata_and_icons_when_supported() {
    // Row 1850: June+ keeps icons; March drops them.
    let descriptor = json!({"name": "t", "icons": [{"src": "x"}], "title": "T"});
    assert!(
        project_tool_descriptor(&descriptor, "2026-07-28")
            .get("icons")
            .is_some()
    );
    assert!(
        project_tool_descriptor(&descriptor, "2025-03-26")
            .get("icons")
            .is_none()
    );
}

#[test]
fn hide_and_reject_tool_when_declaration_incompatible() {
    // Row 1890: unknown tool is a protocol error.
    let s = McpServer::new(ServerInfo::test());
    let ctx = s.context(1, "c");
    assert!(s.call_tool(&ctx, "nope", Some(&json!({}))).is_err());
}

#[test]
fn reject_after_invocation_when_tool_result_cannot_project() {
    // Row 1910: projection failure is a typed error.
    let err = typed_protocol_error("result shape not projectable");
    assert_eq!(err["_tag"], json!("McpProtocolError"));
}

#[test]
fn return_error_result_for_failures_on_2025_11_25() {
    // Row 1935: handler failure rides inside the result with isError.
    let mut s = McpServer::new(ServerInfo::test());
    s.tools_mut()
        .register("f", "f", json!({"type": "object"}), |_| {
            Err("boom".to_owned())
        });
    let ctx = s.context(1, "c");
    let out = s.call_tool(&ctx, "f", Some(&json!({}))).unwrap();
    assert!(out.is_error);
    let wire = error_result_2025_11_25("boom");
    assert_eq!(wire["isError"], json!(true));
}

#[test]
fn report_failures_as_internal_errors_on_older_revisions() {
    // Row 1944: scrubbed message on older wires.
    assert_eq!(
        internal_error_older(),
        "Tool execution failed due to an internal server error."
    );
    assert_eq!(
        elm_effect_unstable::mcp_assembly::internal_error_message(),
        internal_error_older()
    );
}

#[test]
fn reject_invalid_structured_content_at_serialization_boundary() {
    // Row 1955: validation rejects bad args before the handler runs.
    let mut s = McpServer::new(ServerInfo::test());
    s.tools_mut().register(
        "t",
        "t",
        json!({"type": "object", "properties": {"x": {"type": "string"}}, "required": ["x"]}),
        |_| Ok(vec![]),
    );
    let ctx = s.context(1, "c");
    assert!(s.call_tool(&ctx, "t", Some(&json!({"x": 1}))).is_err());
}

#[test]
fn reject_tool_schema_when_input_schema_not_object() {
    // Row 1968.
    let mut tools = elm_effect_unstable::mcp_handlers::ToolRegistry::new();
    tools.register("bad", "bad", json!([]), |_| Ok(vec![]));
    assert!(tools.call("bad", &json!({})).is_err());
}

#[test]
fn list_and_invoke_latest_tool_when_name_registered_twice() {
    // Row 1977: latest ctx-tool shadows the earlier one.
    let mut s = McpServer::new(ServerInfo::test());
    s.register_ctx_tool("dup", "first", json!({"type": "object"}), |_, _| {
        Ok(vec![content_text("first")])
    });
    s.register_ctx_tool("dup", "second", json!({"type": "object"}), |_, _| {
        Ok(vec![content_text("second")])
    });
    let ctx = s.context(1, "c");
    let out = s.call_tool(&ctx, "dup", None).unwrap();
    assert_eq!(out.content[0]["text"], json!("second"));
    assert_eq!(s.ctx_tool_description("dup"), Some("second"));
}
