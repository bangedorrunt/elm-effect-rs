//! MCP version feature-gating witnesses: a full-flags features server
//! advertises neither `completions` nor experimental `tasks`.

use elm_effect_unstable::mcp_server::{CapabilityFlags, ServerCapabilities};

fn features() -> ServerCapabilities {
    ServerCapabilities {
        flags: CapabilityFlags {
            tools: true,
            prompts: true,
            resources: true,
            logging: true,
            sampling: true,
        },
    }
}

#[test]
fn features_server_does_not_advertise_completions() {
    // v2024_11_05.test.ts:38 — completions are absent from the advertisement.
    let advertised = features().advertise();
    assert!(advertised.get("tools").is_some());
    assert!(advertised.get("completions").is_none());
}

#[test]
fn features_server_does_not_expose_experimental_tasks() {
    // v2025_11_25.test.ts:42 — no tasks without the extension.
    let advertised = features().advertise();
    assert!(advertised.get("tools").is_some());
    assert!(!advertised.keys().any(|key| key == "tasks"));
}
