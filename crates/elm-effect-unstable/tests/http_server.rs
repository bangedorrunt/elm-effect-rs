//! Listener-stack fence: explicit bind, route dispatch, 404 fallback.

use elm_effect::Cause;
use elm_effect::Reason;
use elm_effect_unstable::http_server::{
    HttpServer, HttpServerError, Response, RouteError, Router, ServerErrorReason, ServerRequest,
    ServerResponse, cause_response_stripped, is_http_server_error, test_client_origin,
    to_response_or_else,
};
use elm_effect_unstable::net::{NetAddress, NetworkErrorInput};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;

fn get(addr: &std::net::SocketAddr, path: &str) -> (u16, Vec<(String, String)>, Vec<u8>) {
    let mut stream = TcpStream::connect(addr).expect("connect");
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n"
    )
    .expect("write");
    let mut reader = BufReader::new(stream);
    let mut status = String::new();
    reader.read_line(&mut status).expect("status");
    let code: u16 = status
        .split_whitespace()
        .nth(1)
        .expect("code")
        .parse()
        .expect("code");
    let mut headers = Vec::new();
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).expect("header");
        let line = line.trim_end().to_owned();
        if line.is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            headers.push((name.trim().to_owned(), value.trim().to_owned()));
        }
    }
    let mut body = Vec::new();
    use std::io::Read as _;
    reader.read_to_end(&mut body).expect("body");
    (code, headers, body)
}

#[test]
fn binds_loopback_and_serves_one_route() {
    let server = HttpServer::bind("127.0.0.1:0").expect("bind");
    let addr = server.local_addr().expect("addr");
    assert!(addr.ip().is_loopback());
    server.route("GET", "/hello", Response::ok("hello"));
    let handle = std::thread::spawn(move || server.serve_once().expect("serve"));
    let (code, _headers, body) = get(&addr, "/hello");
    assert_eq!(code, 200);
    assert_eq!(body, b"hello");
    assert_eq!(
        handle.join().expect("thread"),
        ("GET".to_owned(), "/hello".to_owned())
    );
}

#[test]
fn unknown_route_is_404() {
    let server = HttpServer::bind("127.0.0.1:0").expect("bind");
    let addr = server.local_addr().expect("addr");
    let handle = std::thread::spawn(move || server.serve_once().expect("serve"));
    let (code, _headers, body) = get(&addr, "/missing");
    assert_eq!(code, 404);
    assert!(body.is_empty());
    assert_eq!(
        handle.join().expect("thread"),
        ("GET".to_owned(), "/missing".to_owned())
    );
}

#[test]
fn explicit_host_is_preserved() {
    let server = HttpServer::bind("127.0.0.1:0").expect("bind");
    let addr = server.local_addr().expect("addr");
    assert_eq!(addr.ip().to_string(), "127.0.0.1");
}

#[test]
fn match_path_extracts_params() {
    use elm_effect_unstable::http_server::match_path;
    assert_eq!(
        match_path("/users/:id", "/users/7"),
        Some(vec![("id".to_string(), "7".to_string())])
    );
    assert_eq!(match_path("/users/:id", "/users/"), None);
    assert_eq!(match_path("/a", "/a/b"), None);
    assert_eq!(
        match_path("/a/:x", "/a/b?x=1#frag"),
        Some(vec![("x".to_string(), "b".to_string())])
    );
}

#[test]
fn patterned_route_serves_with_query_stripped() {
    let server = HttpServer::bind("127.0.0.1:0").expect("bind");
    let addr = server.local_addr().expect("addr");
    server.route("GET", "/users/:id", Response::ok("user"));
    let handle = std::thread::spawn(move || server.serve_once().expect("serve"));
    let (code, _, _) = get(&addr, "/users/7?verbose=1");
    assert_eq!(code, 200);
    let _ = handle.join().expect("join");
}

#[test]
fn post_routes_by_method_not_path_only() {
    use elm_effect_unstable::http_server::{HttpServer, Response};
    let server = HttpServer::bind("127.0.0.1:0").expect("bind");
    let addr = server.local_addr().expect("addr");
    server.route("POST", "/items", Response::ok("created"));
    let handle = std::thread::spawn(move || server.serve_n(2).expect("serve"));
    let (get_code, _, _) = get(&addr, "/items");
    assert_eq!(get_code, 404);
    let (post_code, _, body) = post(&addr, "/items", "application/json", b"{\"a\":1}");
    assert_eq!(post_code, 200);
    assert_eq!(body, b"created");
    let _ = handle.join().expect("join");
}

#[test]
fn response_status_and_headers_preserved() {
    use elm_effect_unstable::http_server::{HttpServer, Response};
    let server = HttpServer::bind("127.0.0.1:0").expect("bind");
    let addr = server.local_addr().expect("addr");
    server.route(
        "GET",
        "/created",
        Response {
            status: 201,
            reason: "Created".to_string(),
            headers: vec![("Content-Type".to_string(), "application/json".to_string())],
            body: b"{}".to_vec(),
        },
    );
    let handle = std::thread::spawn(move || server.serve_once().expect("serve"));
    let (code, headers, _) = get(&addr, "/created");
    assert_eq!(code, 201);
    assert!(
        headers
            .iter()
            .any(|(n, v)| n == "Content-Type" && v == "application/json")
    );
    let _ = handle.join().expect("join");
}

fn post(
    addr: &std::net::SocketAddr,
    path: &str,
    content_type: &str,
    body: &[u8],
) -> (u16, Vec<(String, String)>, Vec<u8>) {
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpStream;
    let mut stream = TcpStream::connect(addr).expect("connect");
    write!(
        stream,
        "POST {path} HTTP/1.1\r\nHost: x\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .expect("write");
    stream.write_all(body).expect("body");
    let mut reader = BufReader::new(stream);
    let mut status = String::new();
    reader.read_line(&mut status).expect("status");
    let code: u16 = status
        .split_whitespace()
        .nth(1)
        .expect("code")
        .parse()
        .expect("code");
    let mut headers = Vec::new();
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).expect("header");
        let line = line.trim_end().to_string();
        if line.is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            headers.push((name.trim().to_string(), value.trim().to_string()));
        }
    }
    let mut response_body = Vec::new();
    use std::io::Read;
    reader.read_to_end(&mut response_body).expect("body");
    (code, headers, response_body)
}

#[test]
fn match_path_ci_static_levels() {
    use elm_effect_unstable::http_server::{match_path, match_path_ci};
    assert_eq!(match_path_ci("/woo", "/WOO"), Some(vec![]));
    assert_eq!(match_path_ci("/foo/bar", "/FOO/BAR"), Some(vec![]));
    assert_eq!(
        match_path_ci("/a/b/c", "/A/B/C"),
        Some(vec![]),
        "three static levels fold case"
    );
    assert_eq!(match_path_ci("/woo", "/nope"), None);
    assert_eq!(match_path_ci("/a/b", "/a/b/c"), None);
}

#[test]
fn match_path_ci_params_keep_path_case() {
    use elm_effect_unstable::http_server::{match_path, match_path_ci};
    assert_eq!(
        match_path_ci("/users/:id", "/USERS/AbC"),
        Some(vec![("id".to_string(), "AbC".to_string())]),
        "static folds, capture keeps original"
    );
    assert_eq!(
        match_path_ci("/API/:Version/items", "/api/v2/ITEMS"),
        Some(vec![("Version".to_string(), "v2".to_string())]),
        "capital pattern folds against lower path"
    );
    assert_eq!(match_path_ci("/users/:id", "/USERS/"), None);
}

#[test]
fn match_path_ci_multiple_routes_first_pattern_wins() {
    use elm_effect_unstable::http_server::{match_path, match_path_ci};
    for path in ["/One/Two", "/ONE/two", "/one/TWO"] {
        assert_eq!(
            match_path_ci("/one/two", path),
            Some(vec![]),
            "every case folds to the same route"
        );
        assert_eq!(match_path("/one/two", path), None, "sensitive keeps case");
    }
    assert_eq!(match_path("/one/two", "/one/two"), Some(vec![]));
}

#[test]
fn match_path_ci_strips_query_and_fragment() {
    use elm_effect_unstable::http_server::{match_path, match_path_ci};
    assert_eq!(
        match_path_ci("/Search/:q", "/SEARCH/Rust?lang=en#top"),
        Some(vec![("q".to_string(), "Rust".to_string())])
    );
}

fn raw(addr: &std::net::SocketAddr, method: &str, path: &str) -> (u16, Vec<(String, String)>, Vec<u8>) {
    let mut stream = TcpStream::connect(addr).expect("connect");
    write!(
        stream,
        "{method} {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n"
    )
    .expect("write");
    let mut reader = BufReader::new(stream);
    let mut status = String::new();
    reader.read_line(&mut status).expect("status");
    let code: u16 = status
        .split_whitespace()
        .nth(1)
        .expect("code")
        .parse()
        .expect("code");
    let mut headers = Vec::new();
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).expect("header");
        let line = line.trim_end().to_owned();
        if line.is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            headers.push((name.trim().to_owned(), value.trim().to_owned()));
        }
    }
    let mut body = Vec::new();
    use std::io::Read as _;
    reader.read_to_end(&mut body).expect("body");
    (code, headers, body)
}

#[test]
fn inherited_method_names_match_nothing() {
    use elm_effect_unstable::http_server::match_path;
    let server = HttpServer::bind("127.0.0.1:0").expect("bind");
    let addr = server.local_addr().expect("addr");
    server.route("GET", "/items", Response::ok("items"));
    let handle = std::thread::spawn(move || server.serve_n(3).expect("serve"));
    // Prototype-inherited names are ordinary unknown methods here: no match.
    for method in ["toString", "constructor", "__proto__"] {
        let (code, _, _) = raw(&addr, method, "/items");
        assert_eq!(code, 404, "{method} must not resolve to a route");
    }
    assert!(match_path("/items", "/items").is_some());
    let _ = handle.join().expect("join");
}

#[test]
fn custom_query_method_dispatches() {
    let server = HttpServer::bind("127.0.0.1:0").expect("bind");
    let addr = server.local_addr().expect("addr");
    server.route("QUERY", "/search", Response::ok("found"));
    let handle = std::thread::spawn(move || server.serve_n(2).expect("serve"));
    let (code, _, body) = raw(&addr, "QUERY", "/search");
    assert_eq!(code, 200);
    assert_eq!(body, b"found");
    let (code, _, _) = raw(&addr, "GET", "/search");
    assert_eq!(code, 404, "other methods do not borrow the QUERY route");
    let _ = handle.join().expect("join");
}

#[test]
fn param_maps_are_own_property_maps() {
    use elm_effect_unstable::http_server::match_path;
    use std::collections::BTreeMap;
    // A parameter literally named like an inherited property stays an own key.
    let captures = match_path("/:toString", "/x").expect("param named toString");
    assert_eq!(captures, vec![("toString".to_string(), "x".to_string())]);
    let map: BTreeMap<_, _> = captures.into_iter().collect();
    assert_eq!(map.get("toString").map(String::as_str), Some("x"));
    assert_eq!(map.get("constructor"), None);
    // Static segments with the same spelling still match exactly.
    assert_eq!(match_path("/toString", "/toString"), Some(vec![]));
    assert_eq!(map.get("hasOwnProperty"), None);
}

#[test]
fn exact_route_beats_pattern_in_dispatch() {
    let server = HttpServer::bind("127.0.0.1:0").expect("bind");
    let addr = server.local_addr().expect("addr");
    server.route("GET", "/users/:id", Response::ok("pattern"));
    server.route("GET", "/users/new", Response::ok("exact"));
    let handle = std::thread::spawn(move || server.serve_n(2).expect("serve"));
    let (code, _, body) = get(&addr, "/users/new");
    assert_eq!(code, 200);
    assert_eq!(body, b"exact", "exact table entry wins over the pattern");
    let (code, _, body) = get(&addr, "/users/7");
    assert_eq!(code, 200);
    assert_eq!(body, b"pattern");
    let _ = handle.join().expect("join");
}

#[test]
fn semicolon_separates_path_from_query() {
    use elm_effect_unstable::http_server::match_path;
    // `;` starts the query, exactly like `?`.
    assert_eq!(match_path("/items", "/items;page=2"), Some(vec![]));
    assert_eq!(match_path("/items/:id", "/items/7;page=2"), Some(vec![("id".to_string(), "7".to_string())]));
    assert_eq!(match_path("/items/:id", "/items/7"), Some(vec![("id".to_string(), "7".to_string())]));
    // A `;` inside a segment is a separator, so it cannot be a literal.
    assert_eq!(match_path("/a/b", "/a;b"), None);
}

#[test]
fn decode_component_rejects_truncated_escapes() {
    use elm_effect_unstable::http_server::decode_component;
    assert_eq!(decode_component("a%20b").as_deref(), Some("a b"));
    assert_eq!(decode_component("%7Bx%7D").as_deref(), Some("{x}"));
    assert_eq!(decode_component("plain+plus").as_deref(), Some("plain+plus"));
    assert_eq!(decode_component("%25").as_deref(), Some("%"));
    // Truncated: escape at the end, and a byte pair cut short.
    assert_eq!(decode_component("abc%"), None);
    assert_eq!(decode_component("abc%2"), None);
    // Non-hex escape.
    assert_eq!(decode_component("%zz"), None);
    // Decoded bytes that are not valid UTF-8 are rejected, not lossy.
    assert_eq!(decode_component("%FF"), None);
}

#[test]
fn wildcard_segment_matches_anything() {
    use elm_effect_unstable::http_server::match_path;
    assert_eq!(match_path("/files/*", "/files/any.txt"), Some(vec![]));
    assert_eq!(match_path("/files/*", "/files/x/y"), None, "wildcard is one segment");
    assert_eq!(
        match_path("/files/*", "/files/"),
        Some(vec![]),
        "wildcard takes an empty segment"
    );
    assert_eq!(match_path("/files/*", "/other"), None);
}

#[test]
fn multi_param_segments_split_on_separators() {
    use elm_effect_unstable::http_server::{match_path_ci, match_path};
    // Case-insensitive forms of FindMyWay's multi-param routes.
    assert_eq!(
        match_path_ci("/:a-:b", "/x-y"),
        Some(vec![
            ("a".to_string(), "x".to_string()),
            ("b".to_string(), "y".to_string())
        ])
    );
    assert_eq!(
        match_path_ci("/Mixed/Case-:v", "/mixed/case-2"),
        Some(vec![("v".to_string(), "2".to_string())]),
        "static part folds, capture keeps original"
    );
    assert_eq!(
        match_path_ci("/:a-:b-:c", "/1-2-3"),
        Some(vec![
            ("a".to_string(), "1".to_string()),
            ("b".to_string(), "2".to_string()),
            ("c".to_string(), "3".to_string())
        ])
    );
}

#[test]
fn multi_param_segments_reject_mismatched_shapes() {
    use elm_effect_unstable::http_server::{match_path, match_path_ci};
    assert_eq!(match_path_ci("/:a-:b", "/nodelimiter"), None);
    assert_eq!(match_path_ci("/:a-:b", "/-"), None, "empty captures rejected");
    assert_eq!(match_path_ci("/:a-:b", "/x-y-extra"), None, "trailing text");
    assert_eq!(match_path_ci("/:a-:b", "/X-Y"), Some(vec![
        ("a".to_string(), "X".to_string()),
        ("b".to_string(), "Y".to_string())
    ]));
}

fn error_request() -> ServerRequest {
    ServerRequest {
        method: "POST".to_string(),
        url: "http://example.com/widgets".to_string(),
    }
}

#[test]
fn server_error_formats_request_reason_messages() {
    // HttpServerError.test.ts:23.
    let parse = ServerErrorReason::RequestParse {
        request: error_request(),
        description: "invalid body".to_string(),
    };
    assert_eq!(
        parse.message(),
        "RequestParseError (POST /widgets): invalid body"
    );
    let missing = ServerErrorReason::RouteNotFound {
        request: error_request(),
    };
    assert_eq!(missing.message(), "RouteNotFound (POST /widgets)");
}

#[test]
fn server_error_formats_response_reason_messages() {
    // HttpServerError.test.ts:35.
    let request = ServerRequest {
        method: "GET".to_string(),
        url: "http://example.com/tea".to_string(),
    };
    let response = ServerResponse { status: 418 };
    let reason = ServerErrorReason::Response {
        request,
        response,
        description: Some("short and stout".to_string()),
    };
    assert_eq!(
        reason.message(),
        "ResponseError (418 GET /tea): short and stout"
    );
}

#[test]
fn server_error_wrapper_delegates() {
    // HttpServerError.test.ts:49 — message, request, response, and
    // reason all delegate to the reason.
    let request = ServerRequest {
        method: "GET".to_string(),
        url: "http://example.com/ok".to_string(),
    };
    let response = ServerResponse { status: 503 };
    let reason = ServerErrorReason::Response {
        request: request.clone(),
        response: response.clone(),
        description: None,
    };
    let error = HttpServerError {
        reason: reason.clone(),
    };
    assert_eq!(error.message(), reason.message());
    assert_eq!(error.message(), "ResponseError (503 GET /ok)");
    assert_eq!(error.request(), &request);
    assert_eq!(error.response(), Some(&response));
    assert_eq!(error.reason, reason);
}

#[test]
fn server_error_forwards_cause_and_maps_status() {
    // HttpServerError.test.ts:61 — the wrapper forwards the reason cause.
    let reason = ServerErrorReason::Internal {
        request: ServerRequest {
            method: "GET".to_string(),
            url: "http://example.com/boom".to_string(),
        },
        cause: "boom".to_string(),
    };
    let error = HttpServerError {
        reason,
    };
    assert_eq!(error.cause(), Some("boom"));
    // HttpServerError.test.ts:73 — reasons map to status codes.
    let request = ServerRequest {
        method: "GET".to_string(),
        url: "http://example.com/status".to_string(),
    };
    let cases = [
        (
            ServerErrorReason::RequestParse {
                request: request.clone(),
                description: "bad".to_string(),
            },
            400,
        ),
        (ServerErrorReason::RouteNotFound { request: request.clone() }, 404),
        (
            ServerErrorReason::Internal {
                request: request.clone(),
                cause: "boom".to_string(),
            },
            500,
        ),
        (
            ServerErrorReason::Response {
                request: request.clone(),
                response: ServerResponse { status: 418 },
                description: None,
            },
            500,
        ),
    ];
    for (reason, status) in cases {
        assert_eq!(reason.status_code(), status);
        assert_eq!(HttpServerError { reason }.status_code(), status);
    }
}

#[test]
fn server_error_guard_matches_wrapper_only() {
    // HttpServerError.test.ts:93 — the guard matches the wrapper.
    // A bare reason is a different type and cannot reach the predicate.
    let error = HttpServerError {
        reason: ServerErrorReason::RouteNotFound {
            request: error_request(),
        },
    };
    assert!(is_http_server_error(&error));
}

#[test]
fn server_error_cause_response_stripped() {
    // HttpServerError.test.ts:107 and :114 — a response carried by a cause
    // becomes the response with nothing left over; anything else yields a
    // 500 and keeps the remaining reasons.
    #[derive(Clone, Debug, PartialEq, Eq)]
    enum HandlerError {
        Respond(Response),
        NotFound(String),
    }
    let classify = |e: &HandlerError| match e {
        HandlerError::Respond(r) => Some(r.clone()),
        HandlerError::NotFound(_) => None,
    };

    // :107 — the only reason is a response: used as-is, no leftover cause.
    let response = Response::empty(418);
    let (out, leftover) = cause_response_stripped(
        Cause::fail(HandlerError::Respond(response.clone())),
        classify,
    );
    assert_eq!(out, response);
    assert!(leftover.is_none());

    // :114 — the failure is not a response: a 500 and the cause survives.
    let cause_in = Cause::fail(HandlerError::NotFound(
        "http://example.com/fail".to_string(),
    ));
    let (out, leftover) = cause_response_stripped(cause_in.clone(), classify);
    assert_eq!(out.status, 500);
    assert_eq!(leftover, Some(cause_in));

    // Mixed: the response reason is stripped and the other one remains.
    let mixed = Cause {
        reasons: vec![
            Reason::Fail(HandlerError::NotFound("x".to_string())),
            Reason::Fail(HandlerError::Respond(Response::empty(418))),
        ],
        annotations: Vec::new(),
    };
    let (out, leftover) = cause_response_stripped(mixed, classify);
    assert_eq!(out.status, 418);
    assert_eq!(leftover.map(|cause| cause.reasons.len()), Some(1));
}

#[test]
fn server_respondable_to_response_or_else_falls_back() {
    // HttpServerRespondable.test.ts:16 — a value that is not a response and
    // carries no protocol implementation gets the fallback. The :7 row
    // (SchemaError -> 400) has no input here: SchemaError lives behind
    // elm-effect's non-default schema feature, which this crate never
    // enables.
    let fallback = Response::empty(500);
    let boom = std::io::Error::other("boom");
    assert_eq!(to_response_or_else(&boom, fallback.clone()), fallback);

    // A response passes through untouched.
    let response = Response::empty(418);
    assert_eq!(to_response_or_else(&response, fallback), response);
}

#[test]
fn test_client_refuses_a_scoped_ipv6_address_before_dropping_the_scope() {
    // HttpServer.test.ts:55 — `makeTestClient` builds the request URL from the
    // server address, so an address carrying a scope id fails there with the
    // address as the error's input. Row :19 shows the formatter dropping a zero
    // scope; this is the case where dropping it would leave the address
    // unreachable, so it must not be formatted at all.
    for input in ["[fe80::1%2]:3000", "[::%2]:3000"] {
        let address = NetAddress::inet_address_from_string_unsafe(input);
        let err = test_client_origin(&address).expect_err("a scoped address is refused");
        assert_eq!(err.tag, "NetAddressError");
        assert_eq!(err.input, NetworkErrorInput::Address(address));
    }
    // The scope id is the whole reason: the same address without it resolves.
    let plain = NetAddress::inet_address_from_string_unsafe("[fe80::1]:3000");
    assert_eq!(
        test_client_origin(&plain).expect("unscoped address"),
        "http://[fe80::1]:3000"
    );
    // An unspecified listener still becomes IPv4 loopback, as in the pin.
    let unspecified = NetAddress::inet_address_from_string_unsafe("[::%0]:8080");
    assert_eq!(
        test_client_origin(&unspecified).expect("unspecified listener"),
        "http://127.0.0.1:8080"
    );
}

#[test]
fn response_set_header_overrides_body_headers() {
    // HttpServerResponse.test.ts:141 — explicit headers win over
    // body-derived content headers.
    use elm_effect_unstable::http_server::Response;
    let mut response = Response::text("body");
    response.set_header("content-type", "text/custom");
    response.set_header("content-length", "1");
    assert_eq!(response.header("content-type"), Some("text/custom"));
    assert_eq!(response.header("content-length"), Some("1"));
    assert_eq!(response.body, b"body");
}

#[test]
fn response_set_body_syncs_metadata_headers() {
    // HttpServerResponse.test.ts:276 — replacing the body drops stale
    // content headers; stream bodies (unknown length) have no
    // representation here, so only the empty/bytes cases pin.
    use elm_effect_unstable::http_server::Response;
    let mut replaced = Response::text("abc");
    replaced.set_body(b"", None);
    assert_eq!(replaced.header("content-type"), None);
    assert_eq!(replaced.header("content-length"), None);
    assert!(replaced.body.is_empty());
    let mut bytes = Response::text("abc");
    bytes.set_body(b"xy", Some("application/octet-stream"));
    assert_eq!(
        bytes.header("content-type"),
        Some("application/octet-stream")
    );
    assert_eq!(bytes.header("content-length"), Some("2"));
}

// FindMyWay router: parameter collisions and node priority.
//
// The pin compiles a pattern into a radix tree and keys its duplicate check on
// a normalized pattern. Handlers here are numbers (or the route's own name), so
// every assertion names the route the pin's tree would have picked.

#[test]
fn router_regex_node_beats_plain_param() {
    // params-collisions.test.ts:4 — a regex node outranks a plain parametric
    // node at the same position.
    let mut router = Router::make();
    assert!(router.on("GET", "/foo/:bar", 1).is_ok());
    assert!(router.on("GET", "/foo/:bar(123)", 2).is_ok());
    assert_eq!(router.find("GET", "/foo/value").expect("plain").handler, 1);
    let regex = router.find("GET", "/foo/123").expect("regex");
    assert_eq!(regex.handler, 2);
    assert_eq!(regex.params, vec![("bar".to_owned(), "123".to_owned())]);
}

#[test]
fn router_static_suffix_beats_plain_param() {
    // params-collisions.test.ts:14 — a node with a static ending outranks the
    // plain node beside it, and captures the text before its own ending.
    let mut router = Router::make();
    assert!(router.on("GET", "/foo/:bar", 1).is_ok());
    assert!(router.on("GET", "/foo/:bar.png", 2).is_ok());
    assert_eq!(router.find("GET", "/foo/value").expect("plain").handler, 1);
    let suffixed = router.find("GET", "/foo/value.png").expect("suffixed");
    assert_eq!(suffixed.handler, 2);
    assert_eq!(
        suffixed.params,
        vec![("bar".to_owned(), "value".to_owned())]
    );
}

#[test]
fn router_rejects_two_plain_param_nodes() {
    // params-collisions.test.ts:24 — two plain parametric nodes erase to one
    // node path, so the second registration is refused.
    let mut router = Router::make();
    assert!(router.on("GET", "/foo/:bar", 1).is_ok());
    let error = router.on("GET", "/foo/:baz", 2).expect_err("collision");
    assert_eq!(
        error,
        RouteError::Duplicate {
            method: "GET".to_owned(),
            pattern: "/foo/:".to_owned(),
        }
    );
    // The first route survives and the refused one never matches.
    assert_eq!(router.find("GET", "/foo/x").expect("plain").handler, 1);
}

#[test]
fn router_rejects_two_regex_nodes() {
    // params-collisions.test.ts:30 — the regex source is erased in the node
    // path, so a second regex node at the same position is a collision too.
    let mut router = Router::make();
    assert!(router.on("GET", "/foo/:bar(123)", 1).is_ok());
    assert!(router.on("GET", "/foo/:bar(456)", 2).is_err());
    assert_eq!(router.find("GET", "/foo/123").expect("regex").handler, 1);
}

#[test]
fn router_distinguishes_static_endings() {
    // params-collisions.test.ts:36 — two static endings are different nodes;
    // a third ending belongs to neither.
    let mut router = Router::make();
    assert!(router.on("GET", "/foo/:bar.png", 1).is_ok());
    assert!(router.on("GET", "/foo/:bar.jpeg", 2).is_ok());
    assert_eq!(router.find("GET", "/foo/value.png").expect("png").handler, 1);
    assert_eq!(
        router.find("GET", "/foo/value.jpeg").expect("jpeg").handler,
        2
    );
    assert!(router.find("GET", "/foo/value.gif").is_none());
}

#[test]
fn router_distinguishes_regex_static_endings() {
    // params-collisions.test.ts:46 — a regex source with a static ending: the
    // source still makes the node a regex node, the ending separates the two.
    let mut router = Router::make();
    assert!(router.on("GET", "/foo/:bar(123).png", 1).is_ok());
    assert!(router.on("GET", "/foo/:bar(456).jpeg", 2).is_ok());
    assert_eq!(router.find("GET", "/foo/123.png").expect("png").handler, 1);
    let jpeg = router.find("GET", "/foo/456.jpeg").expect("jpeg");
    assert_eq!(jpeg.handler, 2);
    assert_eq!(jpeg.params, vec![("bar".to_owned(), "456".to_owned())]);
    // The ending is required, so the bare source alone matches nothing.
    assert!(router.find("GET", "/foo/123").is_none());
}

#[test]
fn router_longer_static_suffix_wins() {
    // params-collisions.test.ts:56 — registered short-first, the longer static
    // suffix is tried first and steps aside when its own ending does not fit.
    let mut router = Router::make();
    assert!(router.on("GET", "/foo/:bar.png", 1).is_ok());
    assert!(router.on("GET", "/foo/:bar.png.png", 2).is_ok());
    assert_eq!(
        router.find("GET", "/foo/value.png").expect("short").handler,
        1
    );
    assert_eq!(
        router.find("GET", "/foo/value.png.png").expect("long").handler,
        2
    );
}

#[test]
fn router_longer_static_suffix_wins_any_order() {
    // params-collisions.test.ts:66 — the same two nodes registered long-first:
    // priority follows the suffix, not the registration order.
    let mut router = Router::make();
    assert!(router.on("GET", "/foo/:bar.png.png", 2).is_ok());
    assert!(router.on("GET", "/foo/:bar.png", 1).is_ok());
    assert_eq!(
        router.find("GET", "/foo/value.png").expect("short").handler,
        1
    );
    let long = router.find("GET", "/foo/value.png.png").expect("long");
    assert_eq!(long.handler, 2);
    assert_eq!(long.params, vec![("bar".to_owned(), "value".to_owned())]);
}

#[test]
fn router_regex_and_static_ending_coexist() {
    // params-collisions.test.ts:76 — a regex node and the longer node that
    // extends it are two nodes; the longer one wins where its ending fits.
    let mut router = Router::make();
    assert!(router.on("GET", "/foo/:bar(123)", 1).is_ok());
    assert!(router.on("GET", "/foo/:bar(123).jpeg", 2).is_ok());
    assert_eq!(router.find("GET", "/foo/123").expect("bare").handler, 1);
    assert_eq!(
        router.find("GET", "/foo/123.jpeg").expect("suffixed").handler,
        2
    );
}

#[test]
fn router_distinguishes_interior_static_parts() {
    // params-collisions.test.ts:86 — routes whose static part sits between two
    // parameters are distinct, and each captures both sides of its separator.
    let mut router = Router::make();
    assert!(router.on("GET", "/foo/:a-:b", "dash").is_ok());
    assert!(router.on("GET", "/foo/:a.:b", "dot").is_ok());
    let dash = router.find("GET", "/foo/x-y").expect("dash");
    assert_eq!(dash.handler, "dash");
    assert_eq!(
        dash.params,
        vec![
            ("a".to_owned(), "x".to_owned()),
            ("b".to_owned(), "y".to_owned())
        ]
    );
    let dot = router.find("GET", "/foo/x.y").expect("dot");
    assert_eq!(dot.handler, "dot");
    assert_eq!(
        dot.params,
        vec![
            ("a".to_owned(), "x".to_owned()),
            ("b".to_owned(), "y".to_owned())
        ]
    );
}
