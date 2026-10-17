//! `packages/effect/test/httpapi/HttpApiEndpoint.test.ts` construction witnesses.

use elm_effect_unstable::http_endpoint::{
    Endpoint, PayloadEncoding, PayloadKind, ResponseBody, ResponseSpec,
};

fn buffered() -> ResponseSpec {
    ResponseSpec::buffered(None, "application/json")
}

fn buffered_ct(content_type: &str) -> ResponseSpec {
    ResponseSpec::buffered(None, content_type)
}

fn sse() -> ResponseSpec {
    ResponseSpec::sse(None, None, None)
}

fn sse_ct(content_type: &str) -> ResponseSpec {
    ResponseSpec::sse(None, Some(content_type), None)
}

fn sse_status(status: u16) -> ResponseSpec {
    ResponseSpec::sse(Some(status), None, None)
}

fn sse_reserved() -> ResponseSpec {
    ResponseSpec::sse(None, None, Some("effect/http-api/stream/failure"))
}

fn bytes_ct(content_type: &str) -> ResponseSpec {
    ResponseSpec::bytes(None, content_type)
}

fn bytes_status(status: u16, content_type: &str) -> ResponseSpec {
    ResponseSpec::bytes(Some(status), content_type)
}

fn nocontent() -> ResponseSpec {
    ResponseSpec::no_content(None)
}

fn nocontent_status(status: u16) -> ResponseSpec {
    ResponseSpec::no_content(Some(status))
}

fn wh(inner: ResponseSpec, status: Option<u16>) -> ResponseSpec {
    ResponseSpec::with_headers(inner, status)
}

fn json_payload(content_type: &str) -> PayloadEncoding {
    PayloadEncoding::new(content_type, PayloadKind::Json)
}

fn text_payload(content_type: &str) -> PayloadEncoding {
    PayloadEncoding::new(content_type, PayloadKind::Text)
}

#[test]
fn stores_the_supplied_identifier() {
    // HttpApiEndpoint.test.ts:14
    let endpoint = Endpoint::get("getUser", "/users/:id").build().unwrap();
    assert_eq!(endpoint.identifier, "getUser");
}

#[test]
fn normalizes_payload_map_keys_while_preserving_the_declared_content_type() {
    // HttpApiEndpoint.test.ts:36
    let endpoint = Endpoint::post("create", "/")
        .payload(vec![json_payload(
            "Application/Vnd.Effect+JSON; Charset=UTF-8",
        )])
        .build()
        .unwrap();
    let entry = endpoint
        .payload_entry("application/vnd.effect+json")
        .unwrap();
    assert_eq!(
        entry.declared_content_type,
        "Application/Vnd.Effect+JSON; Charset=UTF-8"
    );
}

#[test]
fn rejects_incompatible_encodings_for_equivalent_content_types() {
    // HttpApiEndpoint.test.ts:47
    let err = Endpoint::post("create", "/")
        .payload(vec![
            json_payload("Application/Vnd.Effect+Data; charset=utf-8"),
            text_payload("application/vnd.effect+data"),
        ])
        .build()
        .unwrap_err();
    assert!(
        err.contains("Multiple payload encodings"),
        "unexpected: {err}"
    );
}

#[test]
fn get_endpoint_accepts_stream_sse_success() {
    // HttpApiEndpoint.test.ts:63
    let stream = sse();
    let endpoint = Endpoint::get("events", "/events")
        .success(vec![stream.clone()])
        .build()
        .unwrap();
    assert!(endpoint.success_has(&stream));
}

#[test]
fn get_endpoint_accepts_stream_uint8array_success() {
    // HttpApiEndpoint.test.ts:72
    let stream = ResponseSpec::bytes(None, "application/octet-stream");
    let endpoint = Endpoint::get("download", "/download")
        .success(vec![stream.clone()])
        .build()
        .unwrap();
    assert!(endpoint.success_has(&stream));
}

#[test]
fn streaming_schema_in_error_throws_during_endpoint_construction() {
    // HttpApiEndpoint.test.ts:81
    let err = Endpoint::get("events", "/events")
        .error(vec![sse()])
        .build()
        .unwrap_err();
    assert!(
        err.contains("Streaming schemas are not supported in error responses"),
        "unexpected: {err}"
    );
}

#[test]
fn head_with_streaming_success_throws() {
    // HttpApiEndpoint.test.ts:89
    let err = Endpoint::head("events", "/events")
        .success(vec![sse()])
        .build()
        .unwrap_err();
    assert!(
        err.contains("HEAD endpoints cannot declare streaming success responses"),
        "unexpected: {err}"
    );
}

#[test]
fn streaming_success_mixed_with_nocontent_at_the_same_status_throws() {
    // HttpApiEndpoint.test.ts:97
    let err = Endpoint::get("events", "/events")
        .success(vec![sse(), nocontent_status(200)])
        .build()
        .unwrap_err();
    assert!(
        err.contains("Cannot combine no-content and streaming"),
        "unexpected: {err}"
    );
}

#[test]
fn streaming_plus_buffered_at_same_status_allowed_for_distinct_content_types() {
    // HttpApiEndpoint.test.ts:108
    let stream = sse();
    let endpoint = Endpoint::get("events", "/events")
        .success(vec![stream.clone(), buffered()])
        .build()
        .unwrap();
    assert!(endpoint.success_has(&stream));
}

#[test]
fn streaming_plus_buffered_at_same_content_type_throws() {
    // HttpApiEndpoint.test.ts:120
    let err = Endpoint::get("events", "/events")
        .success(vec![sse_ct("application/json"), buffered()])
        .build()
        .unwrap_err();
    assert!(
        err.contains("Cannot combine buffered and streaming"),
        "unexpected: {err}"
    );
}

#[test]
fn streaming_plus_buffered_differing_only_by_parameters_throws() {
    // HttpApiEndpoint.test.ts:131
    let err = Endpoint::get("events", "/events")
        .success(vec![sse_ct("application/json; charset=utf-8"), buffered()])
        .build()
        .unwrap_err();
    assert!(
        err.contains("Cannot combine buffered and streaming"),
        "unexpected: {err}"
    );
}

#[test]
fn streaming_plus_buffered_at_distinct_statuses_is_allowed() {
    // HttpApiEndpoint.test.ts:146 — stream at 206, buffered at 200.
    let stream = sse_status(206);
    let endpoint = Endpoint::get("events", "/events")
        .success(vec![stream.clone(), buffered()])
        .build()
        .unwrap();
    assert!(endpoint.success_has(&stream));
}

#[test]
fn streaming_plus_nocontent_at_distinct_statuses_is_allowed() {
    // HttpApiEndpoint.test.ts:158 — stream at 200, bare NoContent at 204.
    let stream = sse_status(200);
    let endpoint = Endpoint::get("events", "/events")
        .success(vec![stream.clone(), nocontent()])
        .build()
        .unwrap();
    assert!(endpoint.success_has(&stream));
}

#[test]
fn two_streaming_successes_for_the_same_status_throw() {
    // HttpApiEndpoint.test.ts:170
    let err = Endpoint::get("events", "/events")
        .success(vec![sse(), bytes_ct("application/custom-stream")])
        .build()
        .unwrap_err();
    assert!(
        err.contains("Multiple streaming success responses are not supported"),
        "unexpected: {err}"
    );
}

#[test]
fn two_streaming_successes_for_distinct_statuses_throw() {
    // HttpApiEndpoint.test.ts:181
    let err = Endpoint::get("events", "/events")
        .success(vec![
            sse_status(206),
            bytes_status(200, "application/custom-stream"),
        ])
        .build()
        .unwrap_err();
    assert!(
        err.contains("Multiple streaming success responses are not supported"),
        "unexpected: {err}"
    );
}

#[test]
fn statically_detectable_sse_reserved_failure_event_name_throws() {
    // HttpApiEndpoint.test.ts:193
    let err = Endpoint::get("events", "/events")
        .success(vec![sse_reserved()])
        .build()
        .unwrap_err();
    assert!(
        err.contains("SSE event name is reserved"),
        "unexpected: {err}"
    );
}

#[test]
fn rejects_a_withheaders_success_sharing_its_status_and_content_type() {
    // HttpApiEndpoint.test.ts:211
    let err = Endpoint::get("get", "/")
        .success(vec![buffered(), wh(buffered(), None)])
        .build()
        .unwrap_err();
    assert!(
        err.contains("Cannot combine a response with headers"),
        "unexpected: {err}"
    );
}

#[test]
fn rejects_an_encodetowithheaders_error_sharing_its_status_and_content_type() {
    // HttpApiEndpoint.test.ts:227 — encodeToWithHeaders is a headers response.
    let err = Endpoint::get("get", "/")
        .error(vec![buffered(), wh(buffered(), None)])
        .build()
        .unwrap_err();
    assert!(
        err.contains("Cannot combine a response with headers"),
        "unexpected: {err}"
    );
    assert!(err.contains("status 500"), "unexpected: {err}");
}

#[test]
fn rejects_two_withheaders_successes_sharing_a_status_across_content_types() {
    // HttpApiEndpoint.test.ts:250
    let err = Endpoint::get("get", "/")
        .success(vec![
            wh(buffered(), None),
            wh(buffered_ct("text/plain"), None),
        ])
        .build()
        .unwrap_err();
    assert!(
        err.contains("Cannot declare multiple responses with headers for status 200"),
        "unexpected: {err}"
    );
}

#[test]
fn rejects_two_encodetowithheaders_errors_sharing_a_status() {
    // HttpApiEndpoint.test.ts:269
    let err = Endpoint::get("get", "/")
        .error(vec![
            wh(buffered(), None),
            wh(buffered_ct("text/plain"), None),
        ])
        .build()
        .unwrap_err();
    assert!(
        err.contains("Cannot declare multiple responses with headers for status 500"),
        "unexpected: {err}"
    );
}

#[test]
fn rejects_mixed_withheaders_and_encodetowithheaders_responses_sharing_a_status() {
    // HttpApiEndpoint.test.ts:295 — both are headers responses at 429.
    let err = Endpoint::get("get", "/")
        .error(vec![
            wh(buffered_ct("application/json"), Some(429)),
            wh(buffered_ct("text/plain"), Some(429)),
        ])
        .build()
        .unwrap_err();
    assert!(
        err.contains("Cannot declare multiple responses with headers for status 429"),
        "unexpected: {err}"
    );
}

#[test]
fn allows_one_response_with_headers_and_a_plain_response_for_the_same_status_across_content_types()
{
    // HttpApiEndpoint.test.ts:321
    let endpoint = Endpoint::get("get", "/")
        .success(vec![wh(buffered(), None), buffered_ct("text/plain")])
        .build()
        .unwrap();
    assert_eq!(endpoint.success.len(), 2);
}

#[test]
fn allows_responses_with_headers_on_distinct_statuses() {
    // HttpApiEndpoint.test.ts:335
    let endpoint = Endpoint::get("get", "/")
        .success(vec![wh(buffered(), None), wh(buffered(), Some(201))])
        .build()
        .unwrap();
    assert_eq!(endpoint.success.len(), 2);
}

#[test]
fn leaves_the_wrapper_untouched_when_codecs_are_disabled() {
    // HttpApiEndpoint.test.ts:369 — the stored success is the wrapper itself.
    let wrapped = wh(buffered(), None);
    let endpoint = Endpoint::get("list", "/users")
        .disable_codecs()
        .success(vec![wrapped.clone()])
        .build()
        .unwrap();
    assert!(endpoint.success_has(&wrapped));
}

#[test]
fn preserves_disable_codecs_through_derived_endpoints() {
    // HttpApiEndpoint.test.ts:381
    let endpoint = Endpoint::get("list", "/users")
        .disable_codecs()
        .success(vec![buffered()])
        .build()
        .unwrap();
    assert!(endpoint.disable_codecs);
    assert!(endpoint.prefixed("/api").disable_codecs);
    assert!(endpoint.annotated("title", "Users").disable_codecs);
    assert!(
        !Endpoint::get("plain", "/plain")
            .build()
            .unwrap()
            .disable_codecs
    );
}

#[test]
fn keeps_a_wrapped_stream_schema_as_the_inner_schema() {
    // HttpApiEndpoint.test.ts:393
    let stream = sse();
    let endpoint = Endpoint::get("events", "/events")
        .success(vec![wh(stream.clone(), None)])
        .build()
        .unwrap();
    let stored = endpoint.success.iter().find(|r| r.with_headers).unwrap();
    assert_eq!(
        stored.inner_content(),
        &ResponseBody::sse("text/event-stream", None)
    );
    assert_eq!(stored.inner_content(), stream.body().unwrap());
}

#[test]
fn preserves_wrapper_annotations_through_endpoint_construction() {
    // HttpApiEndpoint.test.ts:406 — status(201) on the wrapper wins.
    let endpoint = Endpoint::get("list", "/users")
        .success(vec![wh(buffered(), Some(201))])
        .build()
        .unwrap();
    assert_eq!(endpoint.success[0].status, Some(201));
}

#[test]
fn validates_a_wrapped_stream_like_a_bare_stream() {
    // HttpApiEndpoint.test.ts:417 — wrapped stream counts as a stream twice over.
    let err = Endpoint::get("events", "/events")
        .success(vec![wh(sse(), None), bytes_ct("application/custom-stream")])
        .build()
        .unwrap_err();
    assert!(
        err.contains("Multiple streaming success responses are not supported"),
        "unexpected: {err}"
    );
    let err = Endpoint::head("events", "/events")
        .success(vec![wh(sse(), None)])
        .build()
        .unwrap_err();
    assert!(
        err.contains("HEAD endpoints cannot declare streaming success responses"),
        "unexpected: {err}"
    );
}

#[test]
fn rejects_wrapped_and_bare_streams_at_distinct_statuses() {
    // HttpApiEndpoint.test.ts:433 — one global streaming slot, wrappers unwrap.
    let err = Endpoint::get("events", "/events")
        .success(vec![
            bytes_status(206, "application/custom-bytes"),
            wh(bytes_status(201, "application/other-bytes"), None),
        ])
        .build()
        .unwrap_err();
    assert!(
        err.contains("Multiple streaming success responses are not supported"),
        "unexpected: {err}"
    );
    let err = Endpoint::get("events", "/events")
        .success(vec![wh(sse(), None), sse()])
        .build()
        .unwrap_err();
    assert!(
        err.contains("Multiple streaming success responses are not supported"),
        "unexpected: {err}"
    );
}
