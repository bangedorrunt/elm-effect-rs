//! Multipart portable core: fields, files, limits, errors, schema shape.
//! Ports packages/effect/test/http/Multipart.test.ts rows that do not need
//! the TS Stream/Channel/FileSystem runtime (live backpressure and upstream
//! failure rows are not applicable here).

use elm_effect_unstable::http::{
    MultipartLimits, MultipartPart, MultipartReason, is_multipart_error, multipart_status,
    parse_multipart, persisted_file_json_schema, persisted_file_path, schema_json_with_reviver,
};
use std::collections::BTreeSet;

fn ct(boundary: &str) -> String {
    format!("multipart/form-data; boundary={boundary}")
}

fn part(name: &str, body: &str, boundary: &str) -> String {
    format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{body}\r\n"
    )
}

#[test]
fn schema_json_applies_reviver() {
    let out =
        schema_json_with_reviver(r#"{"value":"original"}"#, |key, value| {
            if key == "value" {
                serde_json::Value::String("revived".to_string())
            } else {
                value
            }
        })
        .expect("parse");
    assert_eq!(out["value"], "revived");
}

#[test]
fn parses_fields_and_file_content() {
    let boundary = "----testboundary";
    let file_bytes = vec![b'A'; 1024 * 16];
    let mut body = Vec::new();
    body.extend_from_slice(part("foo", "bar", boundary).as_bytes());
    body.extend_from_slice(part("test", "ing", boundary).as_bytes());
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"foo.txt\"\r\nContent-Type: text/plain\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(&file_bytes);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());

    let parts = parse_multipart(&body, &ct(boundary), &MultipartLimits::default()).expect("parse");
    assert_eq!(parts.len(), 3);
    assert!(matches!(
        &parts[0],
        MultipartPart::Field { key, value, .. } if key == "foo" && value == "bar"
    ));
    assert!(matches!(
        &parts[1],
        MultipartPart::Field { key, value, .. } if key == "test" && value == "ing"
    ));
    match &parts[2] {
        MultipartPart::File { key: _, name, content, .. } => {
            assert_eq!(name, "foo.txt");
            assert_eq!(content, &file_bytes);
        }
        other => panic!("expected file, got {other:?}"),
    }
    assert!(parts.iter().all(|p| p.is_part() && p.is_stream_part()));
}

#[test]
fn collects_file_across_split_trailing_boundary() {
    let boundary = "----testboundary";
    let chunks: Vec<Vec<u8>> = vec![
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"file.txt\"\r\nContent-Type: text/plain\r\n\r\nabc"
        ).into_bytes(),
        format!("def\r\n--{}", &boundary[..8]).into_bytes(),
        format!("{}--\r\n", &boundary[8..]).into_bytes(),
    ];
    let mut body = Vec::new();
    for chunk in &chunks {
        body.extend_from_slice(chunk);
    }
    let parts = parse_multipart(&body, &ct(boundary), &MultipartLimits::default()).expect("parse");
    assert_eq!(parts.len(), 1);
    match &parts[0] {
        MultipartPart::File { content, .. } => assert_eq!(content, b"abcdef"),
        other => panic!("expected file, got {other:?}"),
    }
}

#[test]
fn parses_non_latin1_filenames() {
    let boundary = "----testboundary";
    let body = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"\u{65e5}\u{672c}\u{8a9e}.txt\"\r\nContent-Type: text/plain\r\n\r\ncontent\r\n--{boundary}--\r\n"
    );
    let parts =
        parse_multipart(body.as_bytes(), &ct(boundary), &MultipartLimits::default()).expect("parse");
    match &parts[0] {
        MultipartPart::File { key, name, .. } => {
            assert_eq!(key, "file");
            assert_eq!(name, "日本語.txt");
        }
        other => panic!("expected file, got {other:?}"),
    }
}

#[test]
fn fails_when_part_limit_exceeded_in_one_chunk() {
    let boundary = "----testboundary";
    let body = part("a", "a", boundary)
        + &part("b", "b", boundary)
        + &part("c", "c", boundary)
        + &format!("--{boundary}--\r\n");
    let err = parse_multipart(
        body.as_bytes(),
        &ct(boundary),
        &MultipartLimits { max_parts: Some(2), ..Default::default() },
    )
    .expect_err("too many parts");
    assert_eq!(err.reason, MultipartReason::TooManyParts);
    assert_eq!(err.status(), 413);
}

#[test]
fn propagates_file_too_large_mid_stream() {
    let boundary = "----testboundary";
    let mut body = Vec::new();
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"file.txt\"\r\nContent-Type: text/plain\r\n\r\na"
        )
        .as_bytes(),
    );
    body.extend_from_slice(&vec![b'a'; 1024]);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    let err = parse_multipart(
        &body,
        &ct(boundary),
        &MultipartLimits { max_file_size: Some(256), ..Default::default() },
    )
    .expect_err("file too large");
    assert_eq!(err.reason, MultipartReason::FileTooLarge);
}

#[test]
fn propagates_body_too_large_mid_file() {
    let boundary = "----testboundary";
    let mut body = Vec::new();
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"file.txt\"\r\nContent-Type: text/plain\r\n\r\na"
        )
        .as_bytes(),
    );
    body.extend_from_slice(&vec![b'a'; 1024]);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    let err = parse_multipart(
        &body,
        &ct(boundary),
        &MultipartLimits { max_total_size: Some(256), ..Default::default() },
    )
    .expect_err("body too large");
    assert_eq!(err.reason, MultipartReason::BodyTooLarge);
}

#[test]
fn propagates_parse_when_body_ends_mid_file() {
    let boundary = "----testboundary";
    let body = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"file.txt\"\r\nContent-Type: text/plain\r\n\r\naBBBB"
    );
    let err = parse_multipart(body.as_bytes(), &ct(boundary), &MultipartLimits::default())
        .expect_err("truncated");
    assert_eq!(err.reason, MultipartReason::Parse);
}

#[test]
fn handles_final_boundary_split_between_trailing_hyphens() {
    let boundary = "----testboundary";
    let first = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"field\"\r\n\r\nvalue\r\n--{boundary}-"
    );
    let second = "-\r\n";
    let mut body = first.into_bytes();
    body.extend_from_slice(second.as_bytes());
    let parts = parse_multipart(&body, &ct(boundary), &MultipartLimits::default()).expect("parse");
    assert_eq!(parts.len(), 1);
    assert!(matches!(
        &parts[0],
        MultipartPart::Field { key, value, .. } if key == "field" && value == "value"
    ));
}

#[test]
fn returns_distinct_persisted_file_paths() {
    let mut used = BTreeSet::new();
    let mut index = 0usize;
    let first = persisted_file_path("/tmp/audit", "same.txt", &mut used, &mut index);
    let second = persisted_file_path("/tmp/audit", "same.txt", &mut used, &mut index);
    assert_eq!(first, "/tmp/audit/same.txt");
    assert_ne!(first, second);
    assert!(used.contains(&first) && used.contains(&second));
}

#[test]
fn responds_by_reason_and_ignores_reporter() {
    for (reason, status) in [
        (MultipartReason::FileTooLarge, 413),
        (MultipartReason::FieldTooLarge, 413),
        (MultipartReason::BodyTooLarge, 413),
        (MultipartReason::TooManyParts, 413),
        (MultipartReason::InternalError, 500),
        (MultipartReason::Parse, 400),
    ] {
        assert_eq!(multipart_status(&reason), status, "{reason:?}");
        assert!(reason.is_ignored());
    }
}

#[test]
fn persisted_file_json_schema_shape() {
    let doc = persisted_file_json_schema();
    let expected = serde_json::json!({
        "dialect": "draft-2020-12",
        "schema": {
            "type": "object",
            "properties": {
                "key": { "type": "string" },
                "name": { "type": "string" },
                "contentType": { "type": "string", "contentEncoding": "binary" },
                "path": { "type": "string" }
            },
            "required": ["key", "name", "contentType", "path"],
            "additionalProperties": true
        },
        "definitions": {}
    });
    assert_eq!(doc, expected);
}

#[test]
fn identifies_multipart_errors() {
    let err = MultipartReason::Parse.error();
    assert!(is_multipart_error(&err));
    assert!(!is_multipart_error(&"MultipartError"));
    assert!(!is_multipart_error(&42_u8));
}
