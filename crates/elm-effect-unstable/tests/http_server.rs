//! Listener-stack fence: explicit bind, route dispatch, 404 fallback.

use elm_effect_unstable::http_server::{
    HttpServer, HttpServerError, Response, ServerErrorReason, ServerRequest, ServerResponse,
    is_http_server_error,
};
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
