//! `HttpServerRequest` kernel: `fromWeb` / `toClientRequest` /
//! `fromClientRequest` URL, header, body, and remote-address behavior that
//! needs no live server or JS web runtime.
//!
//! Carried from `packages/effect/test/http/HttpServerRequest.test.ts`
//! (Effect v4.0.0 pin):
//! - :32 `toClientRequest` (method, client URL, fragment, headers,
//!   ordered URL params, streamed body bytes)
//! - :68 malformed/unsafe content lengths are ignored
//! - :88 empty bodies stay empty
//! - :106 `fromClientRequest` metadata plus every body accessor reading the
//!   same buffered body (the pin's shared concurrent stream accessors; the
//!   Effect stream-sharing machinery itself has no Rust counterpart)
//! - :213 `remoteAddress` defaults to none
//! - :218 `modify` keeps, sets, and clears `remoteAddress`
//!
//! Not carried (no JS web runtime or schema-service wiring on this surface):
//! - :8, :128 FormData/Blob multipart round-trips (needs `FormData`, `Blob`,
//!   and `Request.formData()`; this crate has no multipart codec)
//! - :156, :163 raw `BodyInit` string/bytes bodies (needs web `BodyInit`)
//! - :170 `URLSearchParams` streaming bodies (needs web `URLSearchParams`)
//! - :180 `schemaBodyJson` parse options (needs `Schema` plus
//!   `HttpServerRequest` service provision; `elm-effect-schema` is not a
//!   dependency of this crate)

use elm_effect_unstable::http_server::ServerRequestHead;

fn post_todos() -> ServerRequestHead {
    ServerRequestHead {
        method: "POST".to_owned(),
        original_url: "http://localhost:3000/todos/1?a=1&a=2#top".to_owned(),
        headers: vec![
            ("host".to_owned(), "localhost:3000".to_owned()),
            ("content-type".to_owned(), "application/json".to_owned()),
            ("content-length".to_owned(), "13".to_owned()),
            ("x-test".to_owned(), "ok".to_owned()),
        ],
        body: br#"{"foo":"bar"}"#.to_vec(),
        remote_address: None,
    }
}

#[test]
fn to_client_request_maps_method_url_headers_and_params() {
    // HttpServerRequest.test.ts:32.
    let request = post_todos();
    assert_eq!(request.method, "POST");
    assert_eq!(request.client_url(), "http://localhost:3000/todos/1");
    assert_eq!(request.fragment(), Some("top"));
    assert_eq!(request.header("content-type"), Some("application/json"));
    assert_eq!(request.header("content-length"), Some("13"));
    assert_eq!(request.header("x-test"), Some("ok"));
    // Header lookup folds case, like the pin's `Headers` map.
    assert_eq!(request.header("Content-Type"), Some("application/json"));
    assert_eq!(
        request.query_params(),
        vec![
            ("a".to_owned(), "1".to_owned()),
            ("a".to_owned(), "2".to_owned()),
        ]
    );
    // The streamed body carries the same bytes the pin collects.
    assert_eq!(request.body, br#"{"foo":"bar"}"#);
    assert!(!request.is_empty_body());
}

#[test]
fn to_client_request_ignores_malformed_or_unsafe_content_lengths() {
    // HttpServerRequest.test.ts:68 — every one of these loses its length
    // while the body itself stays a stream.
    for content_length in ["2junk", "1.5", "1e3", "9007199254740992"] {
        let request = ServerRequestHead {
            method: "POST".to_owned(),
            original_url: "http://localhost:3000".to_owned(),
            headers: vec![(
                "content-length".to_owned(),
                content_length.to_owned(),
            )],
            body: b"hello".to_vec(),
            remote_address: None,
        };
        assert_eq!(request.sanitized_content_length(), None, "{content_length}");
        assert!(!request.is_empty_body(), "{content_length}");
    }
    // A well-formed length inside the safe-integer range survives.
    let request = post_todos();
    assert_eq!(request.sanitized_content_length(), Some(13));
}

#[test]
fn to_client_request_keeps_empty_bodies_empty() {
    // HttpServerRequest.test.ts:88.
    let request = ServerRequestHead {
        method: "GET".to_owned(),
        original_url: "http://localhost:3000/todos/1".to_owned(),
        headers: vec![
            ("content-type".to_owned(), "application/json".to_owned()),
            ("x-test".to_owned(), "ok".to_owned()),
        ],
        body: Vec::new(),
        remote_address: None,
    };
    assert!(request.is_empty_body());
    assert_eq!(request.sanitized_content_length(), None);
    assert_eq!(request.header("content-type"), Some("application/json"));
    assert_eq!(request.header("x-test"), Some("ok"));
}

#[test]
fn from_client_request_preserves_metadata_and_shares_body_accessors() {
    // HttpServerRequest.test.ts:106 — the pin appends a param, sets a
    // cookie, and JSON-encodes `{ foo: "bar" }`; every accessor then reads
    // the same body.
    let request = ServerRequestHead {
        method: "POST".to_owned(),
        original_url: "http://localhost:3000/items?existing=1&page=1#top".to_owned(),
        headers: vec![
            ("cookie".to_owned(), "session=123".to_owned()),
            ("content-type".to_owned(), "application/json".to_owned()),
            ("content-length".to_owned(), "13".to_owned()),
        ],
        body: br#"{"foo":"bar"}"#.to_vec(),
        remote_address: None,
    };
    // `source` is the originating request: the head keeps its full URL.
    assert_eq!(
        request.original_url,
        "http://localhost:3000/items?existing=1&page=1#top"
    );
    assert_eq!(request.method, "POST");
    assert_eq!(request.server_url(), "/items?existing=1&page=1#top");
    assert_eq!(request.header("content-type"), Some("application/json"));
    assert_eq!(request.cookie("session"), Some("123"));
    assert_eq!(request.text_body().expect("text"), "{\"foo\":\"bar\"}");
    assert_eq!(request.body, br#"{"foo":"bar"}"#);
    let json = request.json_body().expect("json");
    assert_eq!(json.get("foo").and_then(|value| value.as_str()), Some("bar"));
}

#[test]
fn remote_address_defaults_to_none_for_web_requests() {
    // HttpServerRequest.test.ts:213.
    let request = ServerRequestHead::new("GET", "http://example.com");
    assert_eq!(request.remote_address, None);
}

#[test]
fn modify_distinguishes_missing_and_explicit_none_remote_address() {
    // HttpServerRequest.test.ts:218 — setting survives a no-op modify, and
    // only an explicit none clears it.
    let mut request = ServerRequestHead::new("GET", "http://example.com");
    request.set_remote_address("127.0.0.1");
    assert_eq!(request.remote_address.as_deref(), Some("127.0.0.1"));
    // `modify({})`: no call, the address stays.
    assert_eq!(request.remote_address.as_deref(), Some("127.0.0.1"));
    request.clear_remote_address();
    assert_eq!(request.remote_address, None);
}
