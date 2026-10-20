//! ToolsTest carry rows: registry, list, call, validation (excluding
//! streaming/batch/elicitation-coupled rows, which need the full harness).
use elm_effect_unstable::mcp_handlers::{ToolRegistry, content_text};

fn registry() -> ToolRegistry {
    let mut tools = ToolRegistry::new();
    tools.register(
        "add",
        "add two numbers",
        serde_json::json!({
            "type": "object",
            "properties": { "a": { "type": "number" }, "b": { "type": "number" } },
            "required": ["a", "b"],
        }),
        |args| {
            let a = args["a"].as_f64().unwrap_or(0.0);
            let b = args["b"].as_f64().unwrap_or(0.0);
            Ok(vec![content_text(a + b)])
        },
    );
    tools.register(
        "fail",
        "always fails",
        serde_json::json!({ "type": "object", "properties": {} }),
        |_| Err("boom".to_owned()),
    );
    tools
}

#[test]
fn advertises_when_registered_and_not_when_empty() {
    assert!(registry().capability());
    assert!(!ToolRegistry::new().capability());
}

#[test]
fn list_preserves_names_descriptions_schemas_and_order() {
    let reg = registry();
    let listed = reg.list();
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0].name, "add");
    assert_eq!(listed[0].description, "add two numbers");
    assert_eq!(
        listed[0].input_schema["required"],
        serde_json::json!(["a", "b"])
    );
    assert_eq!(listed[1].name, "fail");
    // stable order across calls
    let again: Vec<_> = registry().list().iter().map(|t| t.name.clone()).collect();
    let once: Vec<_> = listed.iter().map(|t| t.name.clone()).collect();
    assert_eq!(once, again);
}

#[test]
fn calls_with_valid_arguments() {
    let out = registry()
        .call("add", &serde_json::json!({ "a": 2, "b": 3 }))
        .unwrap();
    assert!(!out.is_error);
    assert_eq!(out.content[0]["text"], serde_json::json!(5.0));
}

#[test]
fn rejects_unknown_tool_as_protocol_error() {
    let err = registry().call("nope", &serde_json::json!({})).unwrap_err();
    assert!(err.is_protocol());
}

#[test]
fn returns_handler_failures_with_is_error_not_protocol() {
    let out = registry().call("fail", &serde_json::json!({})).unwrap();
    assert!(out.is_error);
    assert_eq!(out.content[0]["text"], serde_json::json!("boom"));
}

#[test]
fn rejects_missing_required_param_without_invoking_handler() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    let count = Arc::new(AtomicUsize::new(0));
    let moved = count.clone();
    let mut tools = ToolRegistry::new();
    tools.register(
        "t",
        "t",
        serde_json::json!({ "type": "object", "properties": { "x": { "type": "string" } }, "required": ["x"] }),
        move |_| {
            moved.fetch_add(1, Ordering::SeqCst);
            Ok(vec![])
        },
    );
    assert!(tools.call("t", &serde_json::json!({})).is_err());
    assert!(tools.call("t", &serde_json::json!({ "x": 1 })).is_err());
    assert!(tools.call("t", &serde_json::json!("nope")).is_err());
    assert_eq!(count.load(Ordering::SeqCst), 0);
}

#[test]
fn validates_basic_types() {
    let mut tools = ToolRegistry::new();
    tools.register(
        "t",
        "t",
        serde_json::json!({ "type": "object", "properties": {
            "s": { "type": "string" }, "i": { "type": "integer" },
            "b": { "type": "boolean" }, "a": { "type": "array" }, "o": { "type": "object" },
        }}),
        |_| Ok(vec![]),
    );
    let good = serde_json::json!({ "s": "x", "i": 1, "b": true, "a": [], "o": {} });
    assert!(tools.call("t", &good).is_ok());
    for bad in [
        serde_json::json!({ "s": 1, "i": 1, "b": true, "a": [], "o": {} }),
        serde_json::json!({ "s": "x", "i": 1.5, "b": true, "a": [], "o": {} }),
        serde_json::json!({ "s": "x", "i": 1, "b": "y", "a": [], "o": {} }),
        serde_json::json!({ "s": "x", "i": 1, "b": true, "a": {}, "o": {} }),
    ] {
        assert!(tools.call("t", &bad).is_err());
    }
}

#[test]
fn keeps_multiple_content_items_in_order() {
    let mut tools = ToolRegistry::new();
    tools.register("m", "m", serde_json::json!({ "type": "object", "properties": {} }), |_| {
        Ok(vec![
            serde_json::json!({ "type": "text", "text": "one" }),
            serde_json::json!({ "type": "image", "data": "aGk=", "mimeType": "image/png" }),
            serde_json::json!({ "type": "resource", "resource": { "uri": "f://x", "text": "hi" } }),
        ])
    });
    let out = tools.call("m", &serde_json::json!({})).unwrap();
    assert_eq!(out.content.len(), 3);
    assert_eq!(out.content[0]["type"], serde_json::json!("text"));
    assert_eq!(out.content[1]["type"], serde_json::json!("image"));
    assert_eq!(out.content[2]["type"], serde_json::json!("resource"));
}

#[test]
fn rpc_wire_round_trip_for_list_and_call() {
    use elm_effect_unstable::mcp_handlers::{RpcId, RpcRequest, dispatch};
    let tools = registry();
    let req = RpcRequest {
        jsonrpc: "2.0".to_owned(),
        id: Some(RpcId::Num(1)),
        method: "tools/call".to_owned(),
        params: Some(serde_json::json!({ "name": "add", "arguments": { "a": 1, "b": 2 } })),
    };
    let res = dispatch(
        &req,
        &tools,
        &Default::default(),
        &Default::default(),
        &mut Default::default(),
    );
    assert!(res.error.is_none());
    assert_eq!(
        res.result.unwrap()["content"][0]["text"],
        serde_json::json!(3.0)
    );
    let bad = RpcRequest {
        jsonrpc: "2.0".to_owned(),
        id: Some(RpcId::Num(2)),
        method: "tools/call".to_owned(),
        params: Some(serde_json::json!({ "arguments": {} })),
    };
    let res = dispatch(
        &bad,
        &tools,
        &Default::default(),
        &Default::default(),
        &mut Default::default(),
    );
    assert!(res.error.is_some());
}
