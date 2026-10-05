//! RPC wire fence: ndjson framing plus JSON-RPC batch shapes.
//! Mirrors rpc/RpcSerialization.test.ts (framing and batch rows only;
//! socket/HTTP/retry rows need a network runtime).

#[path = "../src/rpc.rs"]
#[allow(dead_code)]
mod rpc;

use rpc::{Json, Message, NdjsonDecoder, decode_batch, encode_batch};

fn request(id: Json, method: &str) -> Message {
    Message::Request {
        id,
        method: method.to_string(),
        payload: Json::Arr(vec![]),
    }
}

#[test]
fn ndjson_reassembles_split_frames() {
    let first = request(Json::Str("a".to_string()), "m");
    let line = first.encode() + "\n";
    let bytes = line.as_bytes();
    let cut = bytes.len() - 2;
    let mut decoder = NdjsonDecoder::new(None);
    assert!(decoder.feed(&bytes[..cut]).unwrap().is_empty());
    assert_eq!(decoder.feed(&bytes[cut..]).unwrap(), vec![first]);
}

#[test]
fn ndjson_decodes_multibyte_split_across_chunks() {
    let msg = Message::Request {
        id: Json::Num("1".to_string()),
        method: "héllo".to_string(),
        payload: Json::Arr(vec![]),
    };
    let bytes = (msg.encode() + "\n").into_bytes();
    let e_acute = "é".as_bytes();
    let split_at = bytes
        .windows(e_acute.len())
        .position(|w| w == e_acute)
        .unwrap()
        + 1;
    let mut decoder = NdjsonDecoder::new(None);
    assert!(decoder.feed(&bytes[..split_at]).unwrap().is_empty());
    assert_eq!(decoder.feed(&bytes[split_at..]).unwrap(), vec![msg]);
}

#[test]
fn ndjson_skips_non_json_lines_and_keeps_decoding() {
    let first = request(Json::Num("1".to_string()), "m");
    let mut decoder = NdjsonDecoder::new(None);
    let fed = format!("not json\n{}\n42\n", first.encode());
    assert_eq!(decoder.feed(fed.as_bytes()).unwrap(), vec![first]);
}

#[test]
fn ndjson_fails_when_unterminated_frame_exceeds_max() {
    let mut decoder = NdjsonDecoder::new(Some(8));
    assert!(decoder.feed(b"123456789").is_err());
}

#[test]
fn ndjson_unbounded_holds_partial_frames() {
    let msg = request(Json::Num("7".to_string()), "m");
    let line = msg.encode() + "\n";
    let mut decoder = NdjsonDecoder::new(None);
    let cut = line.len() / 2;
    assert!(decoder.feed(&line.as_bytes()[..cut]).unwrap().is_empty());
    assert_eq!(decoder.feed(&line.as_bytes()[cut..]).unwrap(), vec![msg]);
}

#[test]
fn ndjson_configured_max_is_honored() {
    let mut decoder = NdjsonDecoder::new(Some(4));
    assert!(decoder.feed(b"12345").is_err());
    let mut open = NdjsonDecoder::new(None);
    assert!(open.feed(b"12345").unwrap().is_empty());
}

#[test]
fn batch_single_encodes_as_object() {
    let msg = request(Json::Str("".to_string()), "m");
    let text = encode_batch(std::slice::from_ref(&msg));
    assert!(text.starts_with('{'));
    assert_eq!(decode_batch(&text), vec![msg]);
}

#[test]
fn batch_multi_encodes_as_array() {
    let first = request(Json::Num("1".to_string()), "a");
    let second = request(Json::Num("2".to_string()), "b");
    let text = encode_batch(&[first.clone(), second.clone()]);
    assert!(text.starts_with('['));
    assert_eq!(decode_batch(&text), vec![first, second]);
}

#[test]
fn batch_decode_skips_non_objects() {
    let msg = request(Json::Num("1".to_string()), "a");
    let text = format!("[{}, 42, \"x\"]", msg.encode());
    assert_eq!(decode_batch(&text), vec![msg]);
}

#[test]
fn batch_preserves_empty_string_id() {
    let msg = request(Json::Str("".to_string()), "m");
    assert_eq!(
        decode_batch(&encode_batch(std::slice::from_ref(&msg))),
        vec![msg]
    );
}
