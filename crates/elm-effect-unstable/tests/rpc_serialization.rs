//! RPC serialization: NDJSON framing, JSON-RPC shapes, cause fidelity.

#[path = "../src/rpc_ser.rs"]
mod rpc_ser;

use rpc_ser::{FramerError, NdjsonDecoder, RpcCause, RpcMessage};
use serde_json::{Value, json};

#[test]
fn ndjson_splits_frames_and_keeps_tail() {
    let mut dec = NdjsonDecoder::new();
    let frames = dec.push(b"{\"a\":1}\n{\"b\":2").unwrap();
    assert_eq!(frames, vec![json!({"a": 1})]);
    let frames = dec.push(b"}\n").unwrap();
    assert_eq!(frames, vec![json!({"b": 2})]);
}

#[test]
fn ndjson_fails_past_max_buffer_without_newline() {
    let mut dec = NdjsonDecoder::with_max_buffer_size(4);
    assert_eq!(dec.push(b"12345"), Err(FramerError::BufferFull));
}

#[test]
fn ndjson_allows_unbounded_incomplete_frames() {
    let mut dec = NdjsonDecoder::new();
    let frames = dec.push(&vec![b'x'; 1024]).unwrap();
    assert!(frames.is_empty());
}

#[test]
fn ndjson_reassembles_multibyte_splits() {
    let mut dec = NdjsonDecoder::new();
    assert!(dec.push("{\"e\":\"é".as_bytes()).unwrap().is_empty());
    let frames = dec.push("\"}\n".as_bytes()).unwrap();
    assert_eq!(frames, vec![json!({"e": "é"})]);
}

#[test]
fn ndjson_skips_non_json_lines() {
    let mut dec = NdjsonDecoder::new();
    let frames = dec.push(b"nope\n{\"a\":1}\nnah\n").unwrap();
    assert_eq!(frames, vec![json!({"a": 1})]);
}

#[test]
fn messages_roundtrip_ids_and_shapes() {
    let req = RpcMessage::Request {
        id: "".to_string(),
        method: "m".to_string(),
        params: json!([1]),
    };
    let back = RpcMessage::decode(&req.encode()).unwrap();
    assert_eq!(back, req);

    let notif = RpcMessage::Notification {
        method: "m".to_string(),
        params: json!(null),
    };
    assert_eq!(RpcMessage::decode(&notif.encode()), Some(notif));

    let resp = RpcMessage::Response {
        id: "7".to_string(),
        result: json!({"ok": true}),
    };
    assert_eq!(RpcMessage::decode(&resp.encode()), Some(resp));
}

#[test]
fn failures_decode_missing_id_as_null() {
    let failure = RpcMessage::Failure {
        id: None,
        code: -32000,
        message: "boom".to_string(),
    };
    let encoded = failure.encode();
    assert_eq!(encoded["id"], Value::Null);
    assert_eq!(RpcMessage::decode(&encoded), Some(failure));

    let typed = RpcMessage::decode(&json!({
        "jsonrpc": "2.0", "id": "9",
        "error": {"code": 1, "message": "nope"},
    }));
    assert!(matches!(typed, Some(RpcMessage::Failure { .. })));
}

#[test]
fn batches_skip_non_objects_and_encode_singly() {
    let batch = vec![
        json!({"jsonrpc": "2.0", "id": "1", "method": "m", "params": null}),
        json!(42),
        json!("junk"),
    ];
    let decoded = RpcMessage::decode_batch(&batch);
    assert_eq!(decoded.len(), 1);

    let single = RpcMessage::encode_responses(&[json!({"id": "1"})]);
    assert!(single.is_object());
    let multi = RpcMessage::encode_responses(&[json!({"id": "1"}), json!({"id": "2"})]);
    assert!(multi.is_array());
}

#[test]
fn causes_roundtrip_every_variant() {
    for cause in [
        RpcCause::Failure("f".to_string()),
        RpcCause::Defect("d".to_string()),
        RpcCause::Interrupt,
        RpcCause::Empty,
    ] {
        assert_eq!(RpcCause::decode(&cause.encode()), Some(cause));
    }
}

#[test]
fn interrupt_without_fiber_encodes_null_fiber_id() {
    // Rpc.test.ts:56 — a missing fiber id is explicit null, never absent.
    assert_eq!(
        RpcCause::Interrupt.encode(),
        json!({"_tag": "Interrupt", "fiberId": null})
    );
    // Decode accepts the null form and the bare legacy form.
    assert_eq!(
        RpcCause::decode(&json!({"_tag": "Interrupt", "fiberId": null})),
        Some(RpcCause::Interrupt)
    );
    assert_eq!(
        RpcCause::decode(&json!({"_tag": "Interrupt"})),
        Some(RpcCause::Interrupt)
    );
    // Re-encoding the decoded value is stable.
    let decoded = RpcCause::decode(&json!({"_tag": "Interrupt"})).unwrap();
    assert_eq!(decoded.encode(), json!({"_tag": "Interrupt", "fiberId": null}));
}
