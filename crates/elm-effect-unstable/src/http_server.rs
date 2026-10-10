//! Minimal HTTP server (`HttpServer` at the pin, first slice).
//!
//! `std::net::TcpListener` bound to an explicit host (IPv4 loopback for
//! tests): one route table plus one response shape. Slice 1 covers bind,
//! single-route dispatch, and 404 fallback; headers/cookies/etag ride the
//! client side (already green there).
//!
//! Residuals: TLS, scoped-IPv6 guards, keep-alive pipelining, chunked
//! request bodies, router middleware, and graceful drain.

use elm_effect::{Cause, Reason};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::{Arc, Mutex};

/// One route: method plus exact path.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Route {
    /// Uppercase method (`"GET"`).
    pub method: String,
    /// Exact path (`"/hello"`).
    pub path: String,
}

/// Response shape.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Response {
    /// Status code.
    pub status: u16,
    /// Reason phrase.
    pub reason: String,
    /// Headers.
    pub headers: Vec<(String, String)>,
    /// Body bytes.
    pub body: Vec<u8>,
}

impl Response {
    /// 200 with a text body.
    pub fn ok(body: &str) -> Self {
        Self {
            status: 200,
            reason: "OK".to_owned(),
            headers: vec![("content-length".to_owned(), body.len().to_string())],
            body: body.as_bytes().to_vec(),
        }
    }

    /// Empty body with an explicit status (`Response.empty`). The reason
    /// phrase is canonical for the codes this port produces, empty for
    /// anything else, so an unknown status never ships a made-up phrase.
    pub fn empty(status: u16) -> Self {
        Self {
            status,
            reason: reason_phrase(status).to_owned(),
            headers: vec![("content-length".to_owned(), "0".to_owned())],
            body: Vec::new(),
        }
    }

    /// 404 with an empty body.
    pub fn not_found() -> Self {
        Self {
            status: 404,
            reason: "Not Found".to_owned(),
            headers: vec![("content-length".to_owned(), "0".to_owned())],
            body: Vec::new(),
        }
    }

    /// 200 with a text body and a `content-type: text/plain` header.
    pub fn text(body: &str) -> Self {
        let mut out = Self::ok(body);
        out.set_header("content-type", "text/plain");
        out
    }

    /// Set a header, replacing any existing entry case-insensitively
    /// (explicit headers override body-derived ones).
    pub fn set_header(&mut self, name: &str, value: &str) {
        self.headers
            .retain(|(n, _)| !n.eq_ignore_ascii_case(name));
        self.headers.push((name.to_owned(), value.to_owned()));
    }

    /// First header value for `name` (case-insensitive), if present.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    /// Replace the body, synchronizing body metadata headers: content
    /// headers drop first; a non-empty body regains `content-length`, and
    /// an explicit content type is set. Stream bodies (unknown length)
    /// have no representation here — use `set_header` directly for those.
    pub fn set_body(&mut self, body: &[u8], content_type: Option<&str>) {
        self.headers.retain(|(n, _)| {
            !n.eq_ignore_ascii_case("content-length")
                && !n.eq_ignore_ascii_case("content-type")
        });
        self.body = body.to_vec();
        if let Some(ct) = content_type {
            self.headers.push(("content-type".to_owned(), ct.to_owned()));
        }
        if !body.is_empty() {
            self.headers
                .push(("content-length".to_owned(), body.len().to_string()));
        }
    }
}

fn reason_phrase(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        _ => "",
    }
}

/// Minimal listener stack: explicit host bind, route table, one-shot serve.
pub struct HttpServer {
    listener: TcpListener,
    routes: Arc<Mutex<HashMap<Route, Response>>>,
}

impl HttpServer {
    /// Bind an explicit host/port (`127.0.0.1:0` picks a free port).
    /// Unspecified hosts stay explicit: callers pass the loopback they mean.
    pub fn bind(addr: &str) -> std::io::Result<Self> {
        Ok(Self {
            listener: TcpListener::bind(addr)?,
            routes: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    /// Local address the listener actually bound (formatted for callbacks).
    pub fn local_addr(&self) -> std::io::Result<SocketAddr> {
        self.listener.local_addr()
    }

    /// Register one route.
    pub fn route(&self, method: &str, path: &str, response: Response) {
        self.routes.lock().expect("server").insert(
            Route {
                method: method.to_owned(),
                path: path.to_owned(),
            },
            response,
        );
    }

    /// Serve exactly one connection, then return.
    pub fn serve_once(&self) -> std::io::Result<(String, String)> {
        let (stream, _) = self.listener.accept()?;
        Ok(handle_one(stream, &self.routes))
    }

    /// Serve `n` connections, returning each request line.
    pub fn serve_n(&self, n: usize) -> std::io::Result<Vec<(String, String)>> {
        let mut out = Vec::with_capacity(n);
        for _ in 0..n {
            let (stream, _) = self.listener.accept()?;
            out.push(handle_one(stream, &self.routes));
        }
        Ok(out)
    }
}

/// Match a route pattern against a request path.
/// `:name` segments capture one non-empty segment; a query string or
/// fragment is stripped before matching. Returns captures in order.
pub fn match_path(pattern: &str, path: &str) -> Option<Vec<(String, String)>> {
    match_path_with(pattern, path, true)
}

/// Case-insensitive route match (FindMyWay `caseSensitive: false`): static
/// segments compare ASCII-case-insensitively while `:param` captures keep
/// the request path's original case.
pub fn match_path_ci(pattern: &str, path: &str) -> Option<Vec<(String, String)>> {
    match_path_with(pattern, path, false)
}

/// Percent-decode one URL component like `decodeURIComponent`: `%XX`
/// becomes the byte, `+` stays a plus. A truncated or non-hex escape is
/// rejected (`None`) rather than passed through, so a malformed segment
/// never silently matches a route.
pub fn decode_component(input: &str) -> Option<String> {
    let bytes = input.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' => {
                if i + 2 >= bytes.len() {
                    return None;
                }
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok()?;
                let byte = u8::from_str_radix(hex, 16).ok()?;
                out.push(byte);
                i += 3;
            }
            byte => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

fn match_path_with(
    pattern: &str,
    path: &str,
    case_sensitive: bool,
) -> Option<Vec<(String, String)>> {
    let path = path.split(['?', '#', ';']).next().unwrap_or(path);
    let pattern_segs: Vec<&str> = pattern.split('/').collect();
    let path_segs: Vec<&str> = path.split('/').collect();
    if pattern_segs.len() != path_segs.len() {
        return None;
    }
    let mut params = Vec::new();
    for (pattern_seg, path_seg) in pattern_segs.iter().zip(path_segs.iter()) {
        if *pattern_seg == "*" {
            // Wildcard: any single segment, empty included.
            continue;
        }
        if pattern_seg.contains(':') && pattern_seg.contains('-') {
            // Multi-parameter segment (`:a-:b`): literal and `:name` parts
            // alternate, separated by `-`; every capture is non-empty.
            let mut rest = *path_seg;
            for (index, part) in pattern_seg.split('-').enumerate() {
                if index > 0 {
                    if !rest.starts_with('-') {
                        return None;
                    }
                    rest = &rest[1..];
                }
                if let Some(name) = part.strip_prefix(':') {
                    let stop = rest.find('-').unwrap_or(rest.len());
                    if stop == 0 {
                        return None;
                    }
                    params.push((name.to_string(), rest[..stop].to_string()));
                    rest = &rest[stop..];
                } else {
                    let matches = if case_sensitive {
                        rest.starts_with(part)
                    } else {
                        rest.len() >= part.len()
                            && rest[..part.len()].eq_ignore_ascii_case(part)
                    };
                    if !matches {
                        return None;
                    }
                    rest = &rest[part.len()..];
                }
            }
            if !rest.is_empty() {
                return None;
            }
            continue;
        }
        if let Some(name) = pattern_seg.strip_prefix(':') {
            if path_seg.is_empty() {
                return None;
            }
            params.push((name.to_string(), (*path_seg).to_string()));
        } else if case_sensitive && pattern_seg != path_seg {
            return None;
        } else if !case_sensitive && !pattern_seg.eq_ignore_ascii_case(path_seg) {
            return None;
        }
    }
    Some(params)
}

fn handle_one(
    stream: TcpStream,
    routes: &Arc<Mutex<HashMap<Route, Response>>>,
) -> (String, String) {
    let mut reader = BufReader::new(stream.try_clone().expect("clone"));
    let mut request_line = String::new();
    reader.read_line(&mut request_line).expect("read line");
    // Drain headers; this slice answers GETs without bodies.
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).expect("read headers");
        if line == "\r\n" || line == "\n" || line.is_empty() {
            break;
        }
    }
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_owned();
    let path = parts.next().unwrap_or("/").to_owned();
    let response = {
        let table = routes.lock().expect("server");
        if let Some(response) = table
            .get(&Route {
                method: method.clone(),
                path: path.clone(),
            })
            .cloned()
        {
            response
        } else {
            // Exact miss: first `:param` pattern for this method wins.
            // ponytail: HashMap scan order; a route Vec if patterns multiply.
            table
                .iter()
                .find(|(route, _)| {
                    route.method == method && match_path(&route.path, &path).is_some()
                })
                .map(|(_, response)| response.clone())
                .unwrap_or_else(Response::not_found)
        }
    };
    let mut writer = stream;
    let mut head = format!("HTTP/1.1 {} {}\r\n", response.status, response.reason).into_bytes();
    for (name, value) in &response.headers {
        head.extend_from_slice(format!("{name}: {value}\r\n").as_bytes());
    }
    head.extend_from_slice(b"connection: close\r\n\r\n");
    head.extend_from_slice(&response.body);
    writer.write_all(&head).expect("write response");
    (method, path)
}

/// Server-side request identity for error reporting.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerRequest {
    /// The request verb (`"POST"`, …).
    pub method: String,
    /// The full request URL.
    pub url: String,
}

/// Server-side response identity for error reporting.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerResponse {
    /// The status code.
    pub status: u16,
}

/// Why a request failed (mirrors `HttpServerError` reason classes).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ServerErrorReason {
    /// The request could not be parsed.
    RequestParse {
        /// The failing request.
        request: ServerRequest,
        /// What was wrong with it.
        description: String,
    },
    /// No route matched.
    RouteNotFound {
        /// The unmatched request.
        request: ServerRequest,
    },
    /// A handler produced a failing response.
    Response {
        /// The failing request.
        request: ServerRequest,
        /// The failing response.
        response: ServerResponse,
        /// What was wrong with it, if stated.
        description: Option<String>,
    },
    /// Anything else, with its cause.
    Internal {
        /// The failing request.
        request: ServerRequest,
        /// The underlying failure.
        cause: String,
    },
}

/// Path part of a URL (`"http://example.com/widgets"` → `"/widgets"`).
fn url_path(url: &str) -> &str {
    let after_scheme = match url.find("://") {
        Some(i) => &url[i + 3..],
        None => return url,
    };
    let path = match after_scheme.find('/') {
        Some(i) => &after_scheme[i..],
        None => "/",
    };
    let path = match path.find(['?', '#']) {
        Some(i) => &path[..i],
        None => path,
    };
    if path.is_empty() {
        "/"
    } else {
        path
    }
}

impl ServerErrorReason {
    /// Human message, mirroring the tag's formats
    /// (`"RequestParseError (POST /widgets): invalid body"`,
    /// `"RouteNotFound (POST /widgets)"`,
    /// `"ResponseError (418 GET /tea): short and stout"`).
    /// `Internal` has no tag-side format pinned, so it follows the same
    /// shape with its cause.
    pub fn message(&self) -> String {
        match self {
            ServerErrorReason::RequestParse {
                request,
                description,
            } => format!(
                "RequestParseError ({} {}): {}",
                request.method,
                url_path(&request.url),
                description
            ),
            ServerErrorReason::RouteNotFound { request } => format!(
                "RouteNotFound ({} {})",
                request.method,
                url_path(&request.url)
            ),
            ServerErrorReason::Response {
                request,
                response,
                description,
            } => {
                let head = format!(
                    "ResponseError ({} {} {})",
                    response.status, request.method,
                    url_path(&request.url)
                );
                match description {
                    Some(d) => format!("{head}: {d}"),
                    None => head,
                }
            }
            ServerErrorReason::Internal { request, cause } => format!(
                "InternalError ({} {}): {}",
                request.method,
                url_path(&request.url),
                cause
            ),
        }
    }

    /// Status code for the failure (`RequestParse` → 400,
    /// `RouteNotFound` → 404, everything else → 500: a failing response
    /// is still a server failure).
    pub fn status_code(&self) -> u16 {
        match self {
            ServerErrorReason::RequestParse { .. } => 400,
            ServerErrorReason::RouteNotFound { .. } => 404,
            ServerErrorReason::Response { .. } | ServerErrorReason::Internal { .. } => 500,
        }
    }
}

/// Server failure wrapper: message, request, response, and cause all
/// delegate to the reason (mirrors `HttpServerError`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpServerError {
    /// The classified failure.
    pub reason: ServerErrorReason,
}

impl HttpServerError {
    /// The reason's message.
    pub fn message(&self) -> String {
        self.reason.message()
    }

    /// The failing request.
    pub fn request(&self) -> &ServerRequest {
        match &self.reason {
            ServerErrorReason::RequestParse { request, .. }
            | ServerErrorReason::RouteNotFound { request }
            | ServerErrorReason::Response { request, .. }
            | ServerErrorReason::Internal { request, .. } => request,
        }
    }

    /// The failing response, when the reason carries one.
    pub fn response(&self) -> Option<&ServerResponse> {
        match &self.reason {
            ServerErrorReason::Response { response, .. } => Some(response),
            _ => None,
        }
    }

    /// The underlying cause, when the reason carries one.
    pub fn cause(&self) -> Option<&str> {
        match &self.reason {
            ServerErrorReason::Internal { cause, .. } => Some(cause),
            _ => None,
        }
    }

    /// The response status for the failure.
    pub fn status_code(&self) -> u16 {
        self.reason.status_code()
    }
}

/// `causeResponseStripped`: reasons that `into_response` maps become the
/// response and are stripped; the remaining reasons survive as a cause, or
/// `None` when nothing is left. No response found yields a 500 response.
///
/// The pin detects a response inside a `Die` defect; a defect here carries
/// only a message, so the classifier closure plays that role instead.
pub fn cause_response_stripped<E>(
    cause: Cause<E>,
    into_response: impl Fn(&E) -> Option<Response>,
) -> (Response, Option<Cause<E>>) {
    let mut response: Option<Response> = None;
    let mut rest: Vec<Reason<E>> = Vec::new();
    for reason in cause.reasons {
        let mapped = match &reason {
            Reason::Fail(value) => into_response(value),
            _ => None,
        };
        match mapped {
            // First response wins; a second one is dropped rather than
            // reported, which is what the pin's `??` assignment does.
            Some(value) => response = response.or(Some(value)),
            None => rest.push(reason),
        }
    }
    let leftover = if rest.is_empty() {
        None
    } else {
        Some(Cause {
            reasons: rest,
            annotations: cause.annotations,
        })
    };
    (response.unwrap_or_else(|| Response::empty(500)), leftover)
}

/// `toResponseOrElse`: a response is used as-is, anything else takes the
/// fallback.
///
/// The pin also maps schema errors to 400 and missing elements to 404;
/// neither type is reachable from this crate (`SchemaError` sits behind
/// elm-effect's non-default `schema` feature), so those arms are absent
/// rather than present and permanently dead.
pub fn to_response_or_else(value: &dyn std::any::Any, or_else: Response) -> Response {
    match value.downcast_ref::<Response>() {
        Some(response) => response.clone(),
        None => or_else,
    }
}

/// Always true for `HttpServerError` (the type already narrows; mirrors
/// `isHttpServerError`, like `is_number` mirrors `isNumber`). A bare
/// reason is a different type and cannot reach this predicate.
pub fn is_http_server_error(_: &HttpServerError) -> bool {
    true
}
