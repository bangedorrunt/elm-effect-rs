//! Phase 1 witnesses: in-memory JSON-RPC framing, version negotiation,
//! initialize handshake, capability advertisement, and ping.
//!
//! TransportsTest rows run over the in-memory channel pair (the local
//! transport); stdio/HTTP/SSE rows are excused (no such transport in
//! phase 1). BaseProtocolTest rows cover framing/handshake/ping; handler
//! rows (toolkit/resource contracts) belong to later lanes.

use elm_effect_unstable::mcp_server::{
    CapabilityFlags, ClientInfo, InMemoryTransport, JsonRpcError, LATEST_VERSION, RequestId,
    SUPPORTED_VERSIONS, ServerCapabilities, ServerInfo, decode_message, encode_message,
};
use serde_json::json;

// --- JSON-RPC 2.0 framing (BaseProtocol > Messages) ---

#[test]
fn accepts_string_and_numeric_request_ids() {
    // BaseProtocolTest statefulLegacy: SCHEMA accepts string/numeric ids.
    let transport = InMemoryTransport::new(ServerCapabilities::minimal(), ServerInfo::test());
    for id in [json!("ping-1"), json!(42)] {
        let raw =
            format!("{{\"jsonrpc\":\"2.0\",\"id\":{id},\"method\":\"ping\",\"params\":{{}}}}");
        let reply = transport.handle_line(&raw).unwrap();
        let msg = decode_message(&reply).unwrap();
        assert_eq!(msg.id(), Some(&RequestId::decode(&id).unwrap()));
        assert!(msg.is_result());
    }
}

#[test]
fn rejects_invalid_jsonrpc_version() {
    // BaseProtocolTest: MUST reject requests with an invalid JSON-RPC version.
    let transport = InMemoryTransport::new(ServerCapabilities::minimal(), ServerInfo::test());
    let reply = transport
        .handle_line(r#"{"jsonrpc":"1.0","id":2,"method":"ping"}"#)
        .unwrap();
    let msg = decode_message(&reply).unwrap();
    assert_eq!(msg.error_code(), Some(JsonRpcError::INVALID_REQUEST));
}

#[test]
fn rejects_invalid_request_ids_with_null_id() {
    // BaseProtocolTest: MUST reject requests with invalid identifiers.
    let transport = InMemoryTransport::new(ServerCapabilities::minimal(), ServerInfo::test());
    let reply = transport
        .handle_line(r#"{"jsonrpc":"2.0","id":true,"method":"ping"}"#)
        .unwrap();
    let msg = decode_message(&reply).unwrap();
    assert_eq!(msg.id(), None);
    assert_eq!(msg.error_code(), Some(JsonRpcError::INVALID_REQUEST));
}

#[test]
fn returns_method_not_found_for_unknown_methods() {
    // BaseProtocolTest: MUST return method not found for unknown methods.
    let transport = InMemoryTransport::initialized(ServerInfo::test());
    let reply = transport
        .handle_line(r#"{"jsonrpc":"2.0","id":3,"method":"unknown/method"}"#)
        .unwrap();
    let msg = decode_message(&reply).unwrap();
    assert_eq!(msg.error_code(), Some(JsonRpcError::METHOD_NOT_FOUND));
}

#[test]
fn returns_invalid_params_for_bad_payloads() {
    // BaseProtocolTest: MUST return invalid params for mismatched payloads.
    let transport = InMemoryTransport::initialized(ServerInfo::test());
    let reply = transport
        .handle_line(r#"{"jsonrpc":"2.0","id":7,"method":"ping","params":"invalid"}"#)
        .unwrap();
    let msg = decode_message(&reply).unwrap();
    assert_eq!(msg.error_code(), Some(JsonRpcError::INVALID_PARAMS));
}

#[test]
fn sends_no_reply_for_notifications() {
    // BaseProtocolTest: MUST not reply to unknown/invalid notifications.
    let transport = InMemoryTransport::initialized(ServerInfo::test());
    assert_eq!(
        transport.handle_line(r#"{"jsonrpc":"2.0","method":"unknown/method"}"#),
        None
    );
    assert_eq!(
        transport.handle_line(r#"{"jsonrpc":"2.0","method":"ping","params":"invalid"}"#),
        None
    );
}

#[test]
fn response_carries_exactly_one_of_result_or_error() {
    // BaseProtocolTest: MUST not include both result and error.
    let transport = InMemoryTransport::initialized(ServerInfo::test());
    let ok = transport
        .handle_line(r#"{"jsonrpc":"2.0","id":9,"method":"ping"}"#)
        .unwrap();
    let ok_msg = decode_message(&ok).unwrap();
    assert!(ok_msg.is_result() && !ok_msg.is_error());
    let err = transport
        .handle_line(r#"{"jsonrpc":"2.0","id":10,"method":"unknown/method"}"#)
        .unwrap();
    let err_msg = decode_message(&err).unwrap();
    assert!(err_msg.is_error() && !err_msg.is_result());
}

#[test]
fn returns_parse_error_for_malformed_json() {
    // BaseProtocolTest: MUST return a parse error for malformed JSON.
    let transport = InMemoryTransport::new(ServerCapabilities::minimal(), ServerInfo::test());
    let reply = transport.handle_line("{").unwrap();
    let msg = decode_message(&reply).unwrap();
    assert_eq!(msg.id(), None);
    assert_eq!(msg.error_code(), Some(JsonRpcError::PARSE_ERROR));
}

#[test]
fn returns_invalid_request_when_method_missing() {
    // BaseProtocolTest: MUST return invalid request for method-less messages.
    let transport = InMemoryTransport::new(ServerCapabilities::minimal(), ServerInfo::test());
    let reply = transport
        .handle_line(r#"{"jsonrpc":"2.0","id":10,"params":{}}"#)
        .unwrap();
    let msg = decode_message(&reply).unwrap();
    assert_eq!(msg.error_code(), Some(JsonRpcError::INVALID_REQUEST));
}

#[test]
fn rejects_jsonrpc_batches() {
    // TransportsTest stdio+HTTP: MUST reject JSON-RPC batches.
    let transport = InMemoryTransport::new(ServerCapabilities::minimal(), ServerInfo::test());
    let reply = transport
        .handle_line(r#"[{"jsonrpc":"2.0","id":2,"method":"ping"},{"jsonrpc":"2.0","id":3,"method":"ping"}]"#)
        .unwrap();
    let msg = decode_message(&reply).unwrap();
    assert_eq!(msg.error_code(), Some(JsonRpcError::INVALID_REQUEST));
}

// --- in-memory transport framing (Transports > local) ---

#[test]
fn frames_are_single_line_compact_json() {
    // TransportsTest stdio: compact newline-delimited records, one per line.
    let (client, _server) = InMemoryTransport::pair(ServerInfo::test());
    let wire = encode_message(&json!({"jsonrpc": "2.0", "id": 1, "method": "ping"}));
    assert!(!wire.contains('\n'));
    client.send_line(&wire).unwrap();
    let frame = client.take_line().unwrap();
    assert!(!frame.contains('\n'));
    let msg = decode_message(&frame).unwrap();
    assert_eq!(msg.id(), Some(&RequestId::Number(1)));
}

#[test]
fn processes_consecutive_messages_independently() {
    // TransportsTest stdio: consecutive messages processed independently.
    let (client, _server) = InMemoryTransport::pair(ServerInfo::test());
    client
        .send_line(r#"{"jsonrpc":"2.0","id":2,"method":"ping"}"#)
        .unwrap();
    client
        .send_line(r#"{"jsonrpc":"2.0","id":3,"method":"ping"}"#)
        .unwrap();
    let first = decode_message(&client.take_line().unwrap()).unwrap();
    let second = decode_message(&client.take_line().unwrap()).unwrap();
    assert_eq!(first.id(), Some(&RequestId::Number(2)));
    assert_eq!(second.id(), Some(&RequestId::Number(3)));
    assert!(first.is_result() && second.is_result());
}

#[test]
fn split_utf8_chunks_reassemble() {
    // TransportsTest stdio: UTF-8 records split across input chunks.
    let (client, _server) = InMemoryTransport::pair(ServerInfo::test());
    let full = format!(
        "{}\n",
        json!({"jsonrpc":"2.0","id":1,"method":"ping","params":{"emoji":"🧪"}})
    );
    let bytes = full.as_bytes();
    let split = bytes.iter().position(|&b| b == 0xf0).unwrap() + 2;
    client.send_chunk(&bytes[..split]).unwrap();
    client.send_chunk(&bytes[split..]).unwrap();
    let frame = client.take_line().unwrap();
    assert_eq!(
        decode_message(&frame).unwrap().id(),
        Some(&RequestId::Number(1))
    );
}

#[test]
fn closing_the_client_shuts_down_cleanly() {
    // TransportsTest stdio: MUST shut down when the client closes stdin.
    let (client, _server) = InMemoryTransport::pair(ServerInfo::test());
    client.close();
    assert!(client.take_line().is_none());
}

// --- version negotiation ---

#[test]
fn negotiates_every_supported_version() {
    // TransportsTest/Lifecycle: 2024-11-05 through 2025-11-25 plus latest.
    assert_eq!(
        SUPPORTED_VERSIONS,
        [
            "2024-11-05",
            "2025-03-26",
            "2025-06-18",
            "2025-11-25",
            "2026-07-28"
        ]
    );
    assert_eq!(LATEST_VERSION, "2026-07-28");
    for version in SUPPORTED_VERSIONS {
        let transport = InMemoryTransport::new(ServerCapabilities::minimal(), ServerInfo::test());
        let reply = transport
            .handle_line(&format!(
                "{{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{{\"protocolVersion\":\"{version}\",\"capabilities\":{{}},\"clientInfo\":{{\"name\":\"c\",\"version\":\"1.0.0\"}}}}}}"
            ))
            .unwrap();
        let msg = decode_message(&reply).unwrap();
        assert!(msg.is_result(), "version {version} refused: {reply}");
        assert!(reply.contains(version));
    }
}

#[test]
fn negotiates_unsupported_versions_down_to_latest() {
    // LifecycleTest: SHOULD negotiate an unsupported version to supported.
    let transport = InMemoryTransport::new(ServerCapabilities::minimal(), ServerInfo::test());
    let reply = transport
        .handle_line(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2099-01-01","capabilities":{},"clientInfo":{"name":"c","version":"1.0.0"}}}"#)
        .unwrap();
    let msg = decode_message(&reply).unwrap();
    assert!(msg.is_result());
    assert!(reply.contains(LATEST_VERSION));
}

// --- initialize handshake + capabilities + ping ---

#[test]
fn initialize_requires_version_capabilities_and_client_info() {
    // LifecycleTest: SCHEMA requires protocolVersion, capabilities, clientInfo.
    let transport = InMemoryTransport::new(ServerCapabilities::minimal(), ServerInfo::test());
    for params in [
        r#"{"capabilities":{},"clientInfo":{"name":"c","version":"1.0.0"}}"#,
        r#"{"protocolVersion":"2025-11-25","clientInfo":{"name":"c","version":"1.0.0"}}"#,
        r#"{"protocolVersion":"2025-11-25","capabilities":{}}"#,
    ] {
        let reply = transport
            .handle_line(&format!(
                "{{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{params}}}"
            ))
            .unwrap();
        assert_eq!(
            decode_message(&reply).unwrap().error_code(),
            Some(JsonRpcError::INVALID_PARAMS),
            "params accepted without required field: {params}"
        );
    }
}

#[test]
fn initialize_returns_capabilities_and_server_info() {
    // LifecycleTest: SCHEMA returns server capabilities + implementation info.
    let caps = ServerCapabilities {
        flags: CapabilityFlags {
            tools: true,
            prompts: true,
            resources: true,
            logging: true,
            sampling: false,
        },
    };
    let transport = InMemoryTransport::new(caps, ServerInfo::new("test-server", "0.1.0"));
    let reply = transport
        .handle_line(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"c","version":"1.0.0"}}}"#)
        .unwrap();
    assert!(reply.contains("\"tools\""));
    assert!(reply.contains("\"prompts\""));
    assert!(reply.contains("\"resources\""));
    assert!(reply.contains("\"logging\""));
    assert!(!reply.contains("\"sampling\""));
    assert!(reply.contains("test-server"));
}

#[test]
fn rejects_non_ping_requests_before_initialize() {
    // LifecycleTest: MUST reject non-ping requests before initialize.
    let transport = InMemoryTransport::new(ServerCapabilities::minimal(), ServerInfo::test());
    let reply = transport
        .handle_line(r#"{"jsonrpc":"2.0","id":3,"method":"tools/list"}"#)
        .unwrap();
    let msg = decode_message(&reply).unwrap();
    assert!(msg.is_error());
    // Ping is allowed pre-initialize so clients can probe liveness.
    let probe = transport
        .handle_line(r#"{"jsonrpc":"2.0","id":4,"method":"ping"}"#)
        .unwrap();
    assert!(decode_message(&probe).unwrap().is_result());
}

#[test]
fn initialized_notification_completes_the_handshake() {
    // LifecycleTest: MUST accept initialized after a successful initialize.
    let transport = InMemoryTransport::new(ServerCapabilities::minimal(), ServerInfo::test());
    transport
        .handle_line(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"c","version":"1.0.0"}}}"#)
        .unwrap();
    assert_eq!(
        transport.handle_line(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#),
        None
    );
    assert!(transport.is_initialized());
}

#[test]
fn rejects_initialized_notification_before_initialize() {
    // LifecycleTest: MUST reject initialized notifications before initialize.
    let transport = InMemoryTransport::new(ServerCapabilities::minimal(), ServerInfo::test());
    assert!(!transport.is_initialized());
    let reply = transport
        .handle_line(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#)
        .unwrap();
    assert_eq!(
        decode_message(&reply).unwrap().error_code(),
        Some(JsonRpcError::INVALID_REQUEST)
    );
    assert!(!transport.is_initialized());
}

#[test]
fn ping_round_trips_after_initialize() {
    let transport = InMemoryTransport::initialized(ServerInfo::test());
    let reply = transport
        .handle_line(r#"{"jsonrpc":"2.0","id":"ping-1","method":"ping"}"#)
        .unwrap();
    let msg = decode_message(&reply).unwrap();
    assert_eq!(
        msg.id(),
        Some(&RequestId::decode(&json!("ping-1")).unwrap())
    );
    assert!(msg.is_result());
}

#[test]
fn advertises_registered_capabilities_only() {
    // LifecycleTest: SCHEMA advertises registered prompt/resource/tool caps.
    let caps = ServerCapabilities {
        flags: CapabilityFlags {
            tools: true,
            prompts: false,
            resources: true,
            logging: false,
            sampling: true,
        },
    };
    let advertised = caps.advertise();
    assert!(advertised.get("tools").is_some());
    assert!(advertised.get("resources").is_some());
    assert!(advertised.get("sampling").is_some());
    assert!(advertised.get("prompts").is_none());
    assert!(advertised.get("logging").is_none());
    let _ = ClientInfo::new("probe", "0.0.1");
}
