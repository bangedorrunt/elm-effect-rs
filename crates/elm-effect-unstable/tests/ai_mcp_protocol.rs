//! `packages/effect/test/ai/McpServer/McpProtocol.test.ts` registry witnesses.

use elm_effect_unstable::mcp_protocol::{ProtocolAdapter, ProtocolRegistry, reverse_error};
use serde_json::json;

fn adapter(version: &str, discriminator: &str) -> ProtocolAdapter {
    ProtocolAdapter::new(version, discriminator)
}

#[test]
fn rejects_an_empty_declaration() {
    // McpProtocol.test.ts:81 — a registry needs at least one protocol.
    let err = ProtocolRegistry::make(vec![]).unwrap_err();
    assert!(err.contains("at least one MCP protocol"), "unexpected: {err}");
}

#[test]
fn rejects_duplicate_versions() {
    // McpProtocol.test.ts:91
    let err = ProtocolRegistry::make(vec![adapter("test-a", "a"), adapter("test-a", "other-a")])
        .unwrap_err();
    assert!(err.contains("Duplicate MCP protocol version: test-a"), "unexpected: {err}");
}

#[test]
fn selects_the_matching_adapter_when_declared() {
    // McpProtocol.test.ts:102
    let registry =
        ProtocolRegistry::make(vec![adapter("test-a", "a"), adapter("test-b", "b")]).unwrap();
    assert_eq!(registry.select("test-b").version, "test-b");
}

#[test]
fn selects_the_first_adapter_when_unsupported() {
    // McpProtocol.test.ts:112
    let registry =
        ProtocolRegistry::make(vec![adapter("test-a", "a"), adapter("test-b", "b")]).unwrap();
    assert_eq!(registry.select("unsupported").version, "test-a");
}

#[test]
fn preserves_declaration_order_when_the_source_mutates() {
    // McpProtocol.test.ts:121 — the registry snapshots at construction.
    let mut protocols = vec![adapter("test-a", "a"), adapter("test-b", "b")];
    let registry = ProtocolRegistry::make(protocols.clone()).unwrap();
    protocols.reverse();
    assert_eq!(registry.select("unsupported").version, "test-a");
    assert_eq!(registry.select("test-b").version, "test-b");
}

#[test]
fn routes_to_only_the_selected_namespace() {
    // McpProtocol.test.ts:137 — incompatible schemas stay in their namespace.
    let registry =
        ProtocolRegistry::make(vec![adapter("test-a", "a"), adapter("test-b", "b")]).unwrap();
    let selected = registry.select("test-a");
    let routed_wrong = selected.route(&json!({"protocol": "b", "value": "wrong adapter"}));
    let other = registry.select("test-b");
    let routed_right = other.route(&json!({"protocol": "b", "value": "other adapter"}));
    assert_ne!(routed_wrong.tag, routed_right.tag);
    assert_ne!(routed_wrong.tag, "test/shape");
    assert!(routed_wrong.decode().is_err());
    assert_eq!(
        routed_right.decode().unwrap(),
        json!({"protocol": "b", "value": "other adapter"})
    );
}

#[test]
fn accepts_resource_links_in_prompt_messages() {
    // McpProtocol.test.ts:175
    let message = elm_effect_unstable::mcp_protocol::decode_prompt_message(&json!({
        "role": "user",
        "content": {"type": "resource_link", "uri": "file:///example.txt", "name": "example"}
    }))
    .unwrap();
    assert_eq!(message.content_kind, "resource_link");
}

#[test]
fn does_not_expose_future_annotations_or_named_extension_capabilities() {
    // McpProtocol.test.ts:188 — unknown fields are dropped on decode.
    let annotations = elm_effect_unstable::mcp_protocol::decode_annotations(&json!({
        "audience": ["user"],
        "lastModified": "2026-07-26"
    }))
    .unwrap();
    assert!(!annotations.contains_key("lastModified"));
    let capabilities = elm_effect_unstable::mcp_protocol::decode_capabilities(&json!({
        "completions": {},
        "extensions": {"example/extension": {"enabled": true}}
    }))
    .unwrap();
    assert!(!capabilities.contains_key("extensions"));
}

#[test]
fn preserves_tagged_unsupported_errors_without_a_local_class() {
    // McpProtocol.test.ts:204 — a tagged value passes through untouched.
    let unsupported = json!({
        "_tag": "McpReverseOperationUnsupported",
        "operation": "roots/list",
        "protocolVersion": "2025-06-18",
        "reason": "Client did not advertise roots"
    });
    assert_eq!(reverse_error("roots/list", unsupported.clone()), unsupported);
    let wrapped = reverse_error("roots/list", json!({"message": "Transport failed"}));
    assert_eq!(wrapped.get("_tag").and_then(|tag| tag.as_str()), Some("McpReverseOperationError"));
}
