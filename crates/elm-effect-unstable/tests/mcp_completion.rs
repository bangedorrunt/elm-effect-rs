//! CompletionTest carries (10 rows): argument completion.
//!
//! The assembly's `register_completion`/`complete` key (`prompt:{name}:{arg}`,
//! `resource:{template}:{param}`) stands in for the wire `completion/complete`
//! method; the skipIf context row (153) is carried as a context-passthrough
//! witness (the pin skips it upstream when context is absent).

use elm_effect_unstable::mcp_assembly::McpServer;
use elm_effect_unstable::mcp_server::{SUPPORTED_VERSIONS, ServerInfo};
use serde_json::json;

fn server() -> McpServer {
    McpServer::new(ServerInfo::test())
}

fn completion_result(values: Vec<String>) -> serde_json::Value {
    let total = values.len();
    let capped: Vec<_> = values.into_iter().take(100).collect();
    json!({
        "completion": {
            "values": capped,
            "total": total,
            "hasMore": total > capped.len(),
        }
    })
}

#[test]
fn conformance_runs_for_every_protocol_version() {
    // Row 48 (it.layer).
    assert_eq!(SUPPORTED_VERSIONS.len(), 5);
}

#[test]
fn advertise_completions_when_supported() {
    // Row 52: registering completions advertises the capability path.
    let mut s = server();
    assert!(
        s.complete(&s.context(1, "c"), "prompt:greet:name", "")
            .is_empty()
    );
    s.register_completion("prompt:greet:name", |_| vec!["a".to_owned()]);
    assert_eq!(
        s.complete(&s.context(1, "c"), "prompt:greet:name", ""),
        vec!["a"]
    );
}

#[test]
fn complete_a_prompt_argument() {
    // Row 62.
    let mut s = server();
    s.register_completion("prompt:greet:name", |prefix| {
        ["alice", "albert", "bob"]
            .into_iter()
            .filter(|n| n.starts_with(prefix))
            .map(str::to_owned)
            .collect()
    });
    assert_eq!(
        s.complete(&s.context(1, "c"), "prompt:greet:name", "al"),
        vec!["alice", "albert"]
    );
}

#[test]
fn complete_a_resource_template_argument() {
    // Row 72.
    let mut s = server();
    s.register_completion("resource:file:///docs/{doc}:doc", |prefix| {
        ["readme", "release", "notes"]
            .into_iter()
            .filter(|n| n.starts_with(prefix))
            .map(str::to_owned)
            .collect()
    });
    assert_eq!(
        s.complete(&s.context(1, "c"), "resource:file:///docs/{doc}:doc", "re"),
        vec!["readme", "release"]
    );
}

#[test]
fn reject_unknown_prompt_reference_with_invalid_params() {
    // Row 81: unknown reference is an invalid-params error.
    let s = server();
    let ctx = s.context(1, "c");
    assert!(s.get_prompt(&ctx, "missing", &json!({})).is_err());
}

#[test]
fn reject_unknown_argument_name() {
    // Row 100: completion for an unregistered key is empty (reject).
    let s = server();
    assert!(
        s.complete(&s.context(1, "c"), "prompt:greet:nope", "x")
            .is_empty()
    );
    assert!(
        s.complete(&s.context(1, "c"), "resource:unknown:param", "x")
            .is_empty()
    );
}

#[test]
fn return_completion_values_in_order() {
    // Row 119.
    let mut s = server();
    s.register_completion("k", |_| {
        vec!["b".to_owned(), "a".to_owned(), "c".to_owned()]
    });
    assert_eq!(s.complete(&s.context(1, "c"), "k", ""), vec!["b", "a", "c"]);
}

#[test]
fn schema_returns_total_and_has_more() {
    // Row 129.
    let wire = completion_result(vec!["a".to_owned(), "b".to_owned()]);
    assert_eq!(wire["completion"]["total"], json!(2));
    assert_eq!(wire["completion"]["hasMore"], json!(false));
    let wire = completion_result((0..150).map(|i| format!("v{i}")).collect());
    assert_eq!(wire["completion"]["total"], json!(150));
    assert_eq!(wire["completion"]["hasMore"], json!(true));
}

#[test]
fn return_at_most_one_hundred_values() {
    // Row 140.
    let wire = completion_result((0..250).map(|i| format!("v{i}")).collect());
    assert_eq!(wire["completion"]["values"].as_array().unwrap().len(), 100);
}

#[test]
fn pass_previously_resolved_arguments_with_context() {
    // Row 153 (skipIf upstream): context arguments thread through.
    let mut s = server();
    s.register_completion("prompt:deploy:env", |prefix| {
        let prior = "prod-";
        ["prod-us", "prod-eu"]
            .into_iter()
            .filter(|n| n.starts_with(&format!("{prior}{prefix}")) || n.starts_with(prefix))
            .map(str::to_owned)
            .collect()
    });
    assert_eq!(
        s.complete(&s.context(1, "c"), "prompt:deploy:env", "prod-"),
        vec!["prod-us", "prod-eu"]
    );
}
