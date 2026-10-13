//! Minimal HTTP server (`HttpServer` at the pin, first slice).
//!
//! `std::net::TcpListener` bound to an explicit host (IPv4 loopback for
//! tests): one route table plus one response shape. Slice 1 covers bind,
//! single-route dispatch, and 404 fallback; headers/cookies/etag ride the
//! client side (already green there).
//!
//! Residuals: TLS, keep-alive pipelining, chunked request bodies, router
//! middleware, and graceful drain.

use crate::net::{NetAddress, NetworkError};
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

/// The origin a test client prefixes requests with, mirroring
/// `HttpServer.makeTestClient`: the address becomes a URL first, so a scoped
/// IPv6 address is refused with the address as the error's input rather than
/// formatted with its scope dropped, since the scope is what makes it
/// reachable. An unspecified listener becomes IPv4 loopback, so the origin
/// points at a host the test can reach.
pub fn test_client_origin(address: &NetAddress) -> Result<String, NetworkError> {
    let url = address.to_url("http")?;
    Ok(match address {
        NetAddress::Inet(socket) if socket.ip().is_unspecified() => match socket.port() {
            80 => "http://127.0.0.1".to_owned(),
            port => format!("http://127.0.0.1:{port}"),
        },
        // `to_url` keeps a default port out of the URL, so only the trailing
        // slash separates it from the origin a client wants.
        _ => url.trim_end_matches('/').to_owned(),
    })
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

// -- FindMyWay router: pattern collisions and node priority ----------------
//
// `FindMyWay.ts` wraps a radix tree built from the pattern string; the key its
// duplicate check compares is a *normalized* pattern, with parameter names and
// regex sources erased and static parts kept. This slice carries that rule plus
// the node ordering `find` relies on, as a flat table rather than a tree:
// registration refuses a repeated key, and a match walks the method's routes in
// priority order (regex nodes first, then the longest static ending) and takes
// the first that fits.
//
// Not carried from the pin: `*` wildcards, the case-insensitive and
// trailing-slash/duplicate-slash options, `::` and `%` escaping in static
// parts, and real regex evaluation (a `(...)` source is compared as the literal
// it holds, which is what every carried row uses).

/// Why a route registration was refused (`Router.on`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RouteError {
    /// Two routes under one method normalize to the same node path; the pin's
    /// message is `Method 'GET' already declared for route '/foo/:'`, so the
    /// key itself is carried as the payload.
    Duplicate { method: String, pattern: String },
    /// The pattern uses a feature this slice does not carry (`*` wildcards).
    Unsupported { pattern: String },
    /// A `(` in the pattern has no matching `)` (`Invalid regexp expression`).
    Invalid { pattern: String },
}

/// A route match: the handler plus the captures in pattern order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found<A> {
    /// The handler registered for the matched route.
    pub handler: A,
    /// Captured parameters, in the order they appear in the pattern.
    pub params: Vec<(String, String)>,
}

/// One piece of a path segment: literal text, or a captured parameter.
#[derive(Clone, Debug, PartialEq, Eq)]
enum SegmentPart {
    /// Literal text, matched byte for byte.
    Literal(String),
    /// A bare `:name` takes the rest of the segment, and an empty segment is no
    /// match (the pin's plain parametric node).
    Plain(String),
    /// A parameter inside a multi-parameter segment (`:a-:b`, `:bar.png`,
    /// `:bar(123)`): the pin compiles these to `(.*?)`, so the capture may be
    /// empty.
    Capture {
        /// Parameter name.
        name: String,
        /// The `(...)` source, compared as a literal.
        regex: Option<String>,
    },
}

/// One `:name` inside a segment, with the static run that follows it.
struct Group {
    name: String,
    regex: Option<String>,
    trailing: String,
}

/// A registered route: the parsed pattern plus its precomputed ordering key.
#[derive(Clone, Debug)]
struct Pattern<A> {
    method: String,
    /// Normalized route key the duplicate check compares.
    key: String,
    segments: Vec<Vec<SegmentPart>>,
    /// The pin's `isRegex` for the node this pattern creates: a regex source or
    /// any static part after a parameter makes the node a regex node.
    is_regex: bool,
    /// Length of the pattern's trailing static ending (the pin's `staticSuffix`).
    static_suffix_len: usize,
    handler: A,
}

impl<A> Pattern<A> {
    fn parse(method: &str, path: &str, handler: A) -> Result<Self, RouteError> {
        let mut key = String::new();
        let mut segments = Vec::new();
        let mut is_regex = false;
        let mut static_suffix_len = 0;
        for (index, segment) in path.split('/').enumerate() {
            if index > 0 {
                key.push('/');
            }
            if segment.contains('*') {
                return Err(RouteError::Unsupported {
                    pattern: path.to_owned(),
                });
            }
            if !segment.contains(':') {
                key.push_str(segment);
                segments.push(vec![SegmentPart::Literal(segment.to_owned())]);
                continue;
            }
            let (head, groups) = parse_segment(segment).ok_or_else(|| RouteError::Invalid {
                pattern: path.to_owned(),
            })?;
            let group_is_regex = groups
                .iter()
                .any(|group| group.regex.is_some() || !group.trailing.is_empty());
            is_regex |= group_is_regex;
            // The pin's node suffix is the trailing static part of the
            // segment's last parameter group.
            if let Some(last) = groups.last() {
                static_suffix_len = last.trailing.len();
            }

            // Key shape: the pin keeps everything up to the first `:`, then
            // writes `()` plus the static part of every group, or nothing at
            // all for a bare parameter.
            key.push_str(&head);
            key.push(':');
            if group_is_regex {
                for group in &groups {
                    key.push_str("()");
                    key.push_str(&group.trailing);
                }
            }

            let mut parts = Vec::new();
            if !head.is_empty() {
                parts.push(SegmentPart::Literal(head));
            }
            let last_group = groups.len().saturating_sub(1);
            for (position, group) in groups.into_iter().enumerate() {
                parts.push(match (&group.regex, position == last_group) {
                    (Some(source), _) => SegmentPart::Capture {
                        name: group.name,
                        regex: Some(source.clone()),
                    },
                    (None, true) if group.trailing.is_empty() => SegmentPart::Plain(group.name),
                    (None, _) => SegmentPart::Capture {
                        name: group.name,
                        regex: None,
                    },
                });
                if !group.trailing.is_empty() {
                    parts.push(SegmentPart::Literal(group.trailing));
                }
            }
            segments.push(parts);
        }
        Ok(Self {
            method: method.to_owned(),
            key,
            segments,
            is_regex,
            static_suffix_len,
            handler,
        })
    }

    /// Match this route against an already-split path.
    fn match_segments(&self, path_segments: &[&str]) -> Option<Vec<(String, String)>> {
        if self.segments.len() != path_segments.len() {
            return None;
        }
        let mut params = Vec::new();
        for (parts, segment) in self.segments.iter().zip(path_segments) {
            params.extend(match_parts(parts, segment)?);
        }
        Some(params)
    }
}

/// Split one segment into its head literal and parameter groups.
fn parse_segment(segment: &str) -> Option<(String, Vec<Group>)> {
    let bytes = segment.as_bytes();
    let mut index = 0;
    while index < bytes.len() && bytes[index] != b':' {
        index += 1;
    }
    let head = segment[..index].to_owned();
    let mut groups = Vec::new();
    while index < bytes.len() {
        index += 1; // the ':' that starts the group
        let name_start = index;
        // A parameter name ends at `(`, `-`, or `.` (end of segment included).
        while index < bytes.len() && !matches!(bytes[index], b'(' | b'-' | b'.') {
            index += 1;
        }
        let name = segment[name_start..index].to_owned();
        let mut regex = None;
        if index < bytes.len() && bytes[index] == b'(' {
            let end = closing_paren(segment, index)?;
            regex = Some(trim_anchors(&segment[index + 1..end]));
            index = end + 1;
        }
        let trailing_start = index;
        while index < bytes.len() && bytes[index] != b':' {
            index += 1;
        }
        groups.push(Group {
            name,
            regex,
            trailing: segment[trailing_start..index].to_owned(),
        });
    }
    Some((head, groups))
}

/// Position of the `)` closing the `(` at `open`, honoring nesting and
/// backslash escapes (`getClosingParenthensePosition`).
fn closing_paren(segment: &str, open: usize) -> Option<usize> {
    let bytes = segment.as_bytes();
    let mut depth = 1usize;
    let mut index = open;
    while index + 1 < bytes.len() {
        index += 1;
        match bytes[index] {
            // A trailing backslash escapes whatever follows, including `)`.
            b'\\' => index += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(index);
                }
            }
            b'(' => depth += 1,
            _ => {}
        }
    }
    None
}

/// Strip a leading `^` and a trailing `$` from a regex source
/// (`trimRegExpStartAndEnd`), so an anchored source compares as the text it
/// pins.
fn trim_anchors(source: &str) -> String {
    let mut text = source;
    if let Some(rest) = text.strip_prefix('^') {
        text = rest;
    }
    if let Some(rest) = text.strip_suffix('$') {
        text = rest;
    }
    text.to_owned()
}

/// Match one segment's parts against one path segment, returning the captures
/// in pattern order.
fn match_parts(parts: &[SegmentPart], rest: &str) -> Option<Vec<(String, String)>> {
    let Some((first, tail)) = parts.split_first() else {
        return rest.is_empty().then(Vec::new);
    };
    match first {
        SegmentPart::Literal(text) => match_parts(tail, rest.strip_prefix(text.as_str())?),
        SegmentPart::Plain(name) => {
            (!rest.is_empty()).then(|| vec![(name.clone(), rest.to_owned())])
        }
        SegmentPart::Capture { name, regex } => match regex {
            // A source compared as the literal it holds: the capture is
            // exactly that text, so one attempt is enough.
            Some(source) => {
                let remainder = rest.strip_prefix(source.as_str())?;
                let mut params = vec![(name.clone(), source.clone())];
                params.extend(match_parts(tail, remainder)?);
                Some(params)
            }
            // Shortest capture first, which is the pin's non-greedy `(.*?)`.
            None => {
                for end in rest
                    .char_indices()
                    .map(|(index, _)| index)
                    .chain([rest.len()])
                {
                    if let Some(mut params) = match_parts(tail, &rest[end..]) {
                        let mut out = vec![(name.clone(), rest[..end].to_owned())];
                        out.append(&mut params);
                        return Some(out);
                    }
                }
                None
            }
        },
    }
}

/// A pattern router (`FindMyWay`): registers routes by method and pattern, and
/// resolves a request path to the highest-priority match.
#[derive(Clone, Debug)]
pub struct Router<A> {
    routes: Vec<Pattern<A>>,
}

impl<A: Clone> Router<A> {
    /// An empty router.
    pub fn make() -> Self {
        Self { routes: Vec::new() }
    }

    /// Register one route (`Router.on`). The pin throws here; the refusal is a
    /// value so the caller decides.
    pub fn on(&mut self, method: &str, path: &str, handler: A) -> Result<(), RouteError> {
        let pattern = Pattern::parse(method, path, handler)?;
        if self
            .routes
            .iter()
            .any(|route| route.method == pattern.method && route.key == pattern.key)
        {
            return Err(RouteError::Duplicate {
                method: pattern.method,
                pattern: pattern.key,
            });
        }
        self.routes.push(pattern);
        Ok(())
    }

    /// Highest-priority route matching `method` and `path`, with its captures
    /// (`Router.find`). The query, fragment, and `;`-separated session id are
    /// stripped before matching, as the pin's `safeDecodeURI` does.
    pub fn find(&self, method: &str, path: &str) -> Option<Found<A>> {
        let path = path.split(['?', '#', ';']).next().unwrap_or(path);
        let segments: Vec<&str> = path.split('/').collect();
        let mut candidates: Vec<&Pattern<A>> = self
            .routes
            .iter()
            .filter(|route| route.method == method)
            .collect();
        // Regex nodes first, then the longest static ending; the sort is
        // stable, so equally ranked routes keep registration order, which is
        // where the pin's own comparator gives up too.
        candidates.sort_by_key(|route| (!route.is_regex, usize::MAX - route.static_suffix_len));
        candidates.into_iter().find_map(|route| {
            route.match_segments(&segments).map(|params| Found {
                handler: route.handler.clone(),
                params,
            })
        })
    }

    /// Whether any route matches (`Router.has`).
    pub fn has(&self, method: &str, path: &str) -> bool {
        self.find(method, path).is_some()
    }
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
