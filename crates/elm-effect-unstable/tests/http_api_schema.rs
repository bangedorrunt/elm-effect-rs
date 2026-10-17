//! `packages/effect/test/httpapi/HttpApiSchema.test.ts` schema-annotation witnesses.

use elm_effect_unstable::http_endpoint::{
    EncodeToWithHeaders, ResponseEncoding, ResponseEncodingTag, SchemaDecl, SchemaMeta, SseMode,
    StreamSseMeta, StreamUint8ArrayMeta, WithHeadersInput, WithHeadersValue, annotate_schema,
    is_stream_schema, is_stream_sse, is_stream_uint8array, is_with_headers,
    response_encoding_schema, status_error_schema, status_success_schema, with_encoding_schema,
    with_headers_schema, with_status_schema,
};
use std::collections::BTreeMap;

fn headers_pair() -> BTreeMap<String, String> {
    BTreeMap::from([("x-a".to_owned(), "1".to_owned())])
}

#[test]
fn stream_sse_stores_default_metadata() {
    // HttpApiSchema.test.ts:21
    let stream = SchemaDecl::StreamSse(StreamSseMeta::events(None));
    assert!(is_stream_schema(&stream));
    assert!(is_stream_sse(&stream));
    assert!(!is_stream_uint8array(&stream));
    let SchemaDecl::StreamSse(meta) = &stream else {
        panic!("expected StreamSse");
    };
    assert_eq!(meta.mode(), "sse");
    assert_eq!(meta.sse_mode, SseMode::Events);
    assert_eq!(meta.content_type, "text/event-stream");
    assert_eq!(meta.error_or_never(), "Never");
}

#[test]
fn stream_sse_stores_custom_content_type() {
    // HttpApiSchema.test.ts:49
    let stream = SchemaDecl::StreamSse(StreamSseMeta::events_ct(
        "text/event-stream; charset=utf-8",
        Some("String"),
    ));
    let SchemaDecl::StreamSse(meta) = &stream else {
        panic!("expected StreamSse");
    };
    assert_eq!(meta.content_type, "text/event-stream; charset=utf-8");
}

#[test]
fn stream_sse_defaults_error_to_never() {
    // HttpApiSchema.test.ts:64
    let stream = SchemaDecl::StreamSse(StreamSseMeta::events(None));
    let SchemaDecl::StreamSse(meta) = &stream else {
        panic!("expected StreamSse");
    };
    assert_eq!(meta.error_or_never(), "Never");
}

#[test]
fn stream_sse_creates_event_schema_from_json_data() {
    // HttpApiSchema.test.ts:74 — the data field is a JSON-string codec.
    #[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
    struct Data {
        id: String,
    }
    let stream = SchemaDecl::StreamSse(StreamSseMeta::data("Data", None));
    let SchemaDecl::StreamSse(meta) = &stream else {
        panic!("expected StreamSse");
    };
    assert_eq!(meta.sse_mode, SseMode::Data);
    let encoded = StreamSseMeta::encode_data(
        "user.created",
        &Data {
            id: "123".to_owned(),
        },
    )
    .unwrap();
    assert_eq!(encoded.id, None);
    assert_eq!(encoded.event, "user.created");
    assert_eq!(encoded.data_json, r#"{"id":"123"}"#);
    let back: Data = StreamSseMeta::decode_data(&encoded).unwrap();
    assert_eq!(
        back,
        Data {
            id: "123".to_owned()
        }
    );
}

#[test]
fn stream_uint8array_stores_default_metadata() {
    // HttpApiSchema.test.ts:94
    let stream = SchemaDecl::StreamBytes(StreamUint8ArrayMeta::new());
    assert!(is_stream_schema(&stream));
    assert!(!is_stream_sse(&stream));
    assert!(is_stream_uint8array(&stream));
    let SchemaDecl::StreamBytes(meta) = &stream else {
        panic!("expected StreamBytes");
    };
    assert_eq!(meta.mode(), "uint8array");
    assert_eq!(meta.content_type, "application/octet-stream");
}

#[test]
fn stream_uint8array_stores_custom_content_type() {
    // HttpApiSchema.test.ts:109
    let stream = SchemaDecl::StreamBytes(StreamUint8ArrayMeta::with_content_type(
        "application/custom-binary",
    ));
    let SchemaDecl::StreamBytes(meta) = &stream else {
        panic!("expected StreamBytes");
    };
    assert_eq!(meta.content_type, "application/custom-binary");
}

#[test]
fn with_headers_stores_inner_and_headers() {
    // HttpApiSchema.test.ts:119
    let inner = SchemaDecl::Plain(SchemaMeta::plain());
    let schema = with_headers_schema(&inner, &["x-total-count"]).unwrap();
    assert!(is_with_headers(&schema));
    let SchemaDecl::WithHeaders(wrapped) = &schema else {
        panic!("expected WithHeaders");
    };
    assert_eq!(*wrapped.inner, inner);
    assert_eq!(wrapped.header_names, vec!["x-total-count".to_owned()]);
}

#[test]
fn does_not_identify_other_schemas_as_with_headers() {
    // HttpApiSchema.test.ts:129
    assert!(!is_with_headers(&SchemaDecl::Plain(SchemaMeta::plain())));
    assert!(!is_with_headers(&SchemaDecl::StreamBytes(
        StreamUint8ArrayMeta::new()
    )));
}

#[test]
fn with_headers_supports_fields_shorthand() {
    // HttpApiSchema.test.ts:134 — a bare field list becomes the headers record.
    let schema =
        with_headers_schema(&SchemaDecl::Plain(SchemaMeta::plain()), &["x-total-count"]).unwrap();
    let SchemaDecl::WithHeaders(wrapped) = &schema else {
        panic!("expected WithHeaders");
    };
    assert!(!wrapped.header_names.is_empty());
    assert_eq!(wrapped.header_names, vec!["x-total-count".to_owned()]);
}

#[test]
fn with_headers_rejects_nested_wrappers() {
    // HttpApiSchema.test.ts:143
    let inner = with_headers_schema(&SchemaDecl::Plain(SchemaMeta::plain()), &["x-a"]).unwrap();
    let err = with_headers_schema(&inner, &["x-b"]).unwrap_err();
    assert!(
        err.contains("WithHeaders schemas cannot be nested"),
        "unexpected: {err}"
    );
}

#[test]
fn with_headers_accepts_only_branded_values() {
    // HttpApiSchema.test.ts:152
    let value = WithHeadersValue::new("a".to_owned(), headers_pair());
    let ok = WithHeadersValue::decode(WithHeadersInput::Branded(value.clone())).unwrap();
    assert_eq!(ok, value);
    let err = WithHeadersValue::decode(WithHeadersInput::Plain {
        body: "a".to_owned(),
        headers: headers_pair(),
    })
    .unwrap_err();
    assert!(err.contains("branded"), "unexpected: {err}");
}

#[test]
fn with_headers_keeps_wrapper_intact_through_annotate() {
    // HttpApiSchema.test.ts:160
    let schema = with_headers_schema(&SchemaDecl::Plain(SchemaMeta::plain()), &["x-a"]).unwrap();
    let annotated = annotate_schema(&schema, "described");
    assert!(is_with_headers(&annotated));
    let SchemaDecl::WithHeaders(wrapped) = &annotated else {
        panic!("expected WithHeaders");
    };
    assert_eq!(*wrapped.inner, SchemaDecl::Plain(SchemaMeta::plain()));
    assert_eq!(wrapped.header_names, vec!["x-a".to_owned()]);
}

#[test]
fn with_headers_resolves_status_wrapper_first() {
    // HttpApiSchema.test.ts:169
    let headers = ["x-a"];
    let on_inner = with_headers_schema(
        &SchemaDecl::Plain(SchemaMeta::plain().status(201)),
        &headers,
    )
    .unwrap();
    assert_eq!(status_success_schema(&on_inner), 201);

    let on_wrapper =
        with_headers_schema(&SchemaDecl::Plain(SchemaMeta::plain()), &headers).unwrap();
    let on_wrapper = with_status_schema(&on_wrapper, 202);
    assert!(is_with_headers(&on_wrapper));
    assert_eq!(status_success_schema(&on_wrapper), 202);

    let on_both = with_headers_schema(
        &SchemaDecl::Plain(SchemaMeta::plain().status(201)),
        &headers,
    )
    .unwrap();
    let on_both = with_status_schema(&on_both, 202);
    assert_eq!(status_success_schema(&on_both), 202);

    let on_neither =
        with_headers_schema(&SchemaDecl::Plain(SchemaMeta::plain()), &headers).unwrap();
    assert_eq!(status_success_schema(&on_neither), 200);

    assert_eq!(
        status_success_schema(&SchemaDecl::Plain(SchemaMeta::plain().status(201))),
        201
    );
}

#[test]
fn with_headers_resolves_encoding_wrapper_first() {
    // HttpApiSchema.test.ts:189
    let headers = ["x-a"];
    let on_inner =
        with_headers_schema(&SchemaDecl::Plain(SchemaMeta::plain().as_text()), &headers).unwrap();
    assert_eq!(
        response_encoding_schema(&on_inner).tag,
        ResponseEncodingTag::Text
    );

    let on_wrapper =
        with_headers_schema(&SchemaDecl::Plain(SchemaMeta::plain()), &headers).unwrap();
    let on_wrapper = with_encoding_schema(
        &on_wrapper,
        ResponseEncoding::json_ct("application/vnd.custom+json"),
    );
    assert!(is_with_headers(&on_wrapper));
    assert_eq!(
        response_encoding_schema(&on_wrapper).content_type,
        "application/vnd.custom+json"
    );

    let on_neither =
        with_headers_schema(&SchemaDecl::Plain(SchemaMeta::plain()), &headers).unwrap();
    assert_eq!(
        response_encoding_schema(&on_neither).tag,
        ResponseEncodingTag::Json
    );
}

#[test]
fn with_headers_value_constructs_branded_response() {
    // HttpApiSchema.test.ts:210
    let value = WithHeadersValue::new("a".to_owned(), headers_pair());
    assert_eq!(value.body, "a");
    assert_eq!(value.headers, headers_pair());
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct UserNotFound {
    user_id: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct UserIdHeaders {
    user_id: i32,
}

fn user_not_found_codec() -> EncodeToWithHeaders<UserNotFound, (), UserIdHeaders> {
    EncodeToWithHeaders::new(
        SchemaMeta::empty(404),
        &["x-user-id"],
        |error: &UserNotFound| {
            (
                (),
                UserIdHeaders {
                    user_id: error.user_id,
                },
            )
        },
        |(_body, headers): ((), UserIdHeaders)| UserNotFound {
            user_id: headers.user_id,
        },
    )
}

#[test]
fn encode_to_with_headers_encodes_body_and_headers_pair() {
    // HttpApiSchema.test.ts:236
    let codec = user_not_found_codec();
    assert_eq!(codec.header_names, vec!["x-user-id".to_owned()]);
    assert_eq!(
        codec.encode(&UserNotFound { user_id: 1 }),
        ((), UserIdHeaders { user_id: 1 })
    );
}

#[test]
fn encode_to_with_headers_decodes_pair_back() {
    // HttpApiSchema.test.ts:243
    let codec = user_not_found_codec();
    assert_eq!(
        codec.decode((), UserIdHeaders { user_id: 1 }),
        UserNotFound { user_id: 1 }
    );
}

#[test]
fn encode_to_with_headers_resolves_status_from_body() {
    // HttpApiSchema.test.ts:250
    assert_eq!(user_not_found_codec().status_error(), 404);
}

#[test]
fn encode_to_with_headers_resolves_encoding_from_body() {
    // HttpApiSchema.test.ts:254
    let codec = EncodeToWithHeaders::new(
        SchemaMeta::plain().as_text(),
        &["x-a"],
        |body: &String| (body.clone(), headers_pair()),
        |(body, _headers): (String, BTreeMap<String, String>)| body,
    );
    assert_eq!(codec.response_encoding().tag, ResponseEncodingTag::Text);
}

#[test]
fn encode_to_with_headers_defaults_encoding_to_json() {
    // HttpApiSchema.test.ts:268
    assert_eq!(
        user_not_found_codec().response_encoding().tag,
        ResponseEncodingTag::Json
    );
}

#[test]
fn encode_to_with_headers_uses_body_defaults_not_source_metadata() {
    // HttpApiSchema.test.ts:272 — source status/encoding never reach the pair.
    let _source = SchemaMeta::plain().status(429).as_text();
    let codec = EncodeToWithHeaders::new(
        SchemaMeta::plain(),
        &["x-note"],
        |body: &String| (body.clone(), "test".to_owned()),
        |(body, _headers): (String, String)| body,
    );
    assert_eq!(codec.status_error(), 500);
    assert_eq!(codec.response_encoding(), ResponseEncoding::json());
}

#[test]
fn does_not_identify_buffered_as_stream() {
    // HttpApiSchema.test.ts:295
    assert!(!is_stream_schema(&SchemaDecl::Plain(SchemaMeta::plain())));
    assert!(!is_stream_schema(&SchemaDecl::Plain(
        SchemaMeta::plain().as_uint8array()
    )));
}

#[test]
fn status_accepts_literals() {
    // HttpApiSchema.test.ts:301
    assert_eq!(
        status_success_schema(&SchemaDecl::Plain(
            SchemaMeta::plain().status_literal("Created")
        )),
        201
    );
    assert_eq!(
        status_error_schema(&SchemaDecl::Plain(
            SchemaMeta::plain().status_literal("Conflict")
        )),
        409
    );
}
