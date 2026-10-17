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
        self.headers.retain(|(n, _)| !n.eq_ignore_ascii_case(name));
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
            !n.eq_ignore_ascii_case("content-length") && !n.eq_ignore_ascii_case("content-type")
        });
        self.body = body.to_vec();
        if let Some(ct) = content_type {
            self.headers
                .push(("content-type".to_owned(), ct.to_owned()));
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
/// `:name` segments capture one non-empty segment; `:name` followed by
/// literal text (an action suffix such as the `:wait` in
/// `/operations/:id:wait`) captures the head and matches the tail
/// literally. A query string or fragment is stripped before matching.
/// Returns captures in order.
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
                        rest.len() >= part.len() && rest[..part.len()].eq_ignore_ascii_case(part)
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
        if let Some(after) = pattern_seg.strip_prefix(':') {
            let name_len = after
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                .unwrap_or(after.len());
            let (name, suffix) = after.split_at(name_len);
            // ponytail: one leading `:name` plus one literal tail; a `?`
            // optional marker keeps the old whole-segment binding.
            if !name.is_empty() && !suffix.is_empty() && !suffix.starts_with('?') {
                // `get` (not slicing): a suffix boundary inside a char is a miss, not a panic.
                let tail = path_seg.get(path_seg.len().saturating_sub(suffix.len())..);
                let Some(tail) = tail.filter(|tail| tail.len() == suffix.len()) else {
                    return None;
                };
                let suffix_ok = if case_sensitive {
                    tail == suffix
                } else {
                    tail.eq_ignore_ascii_case(suffix)
                };
                if !suffix_ok {
                    return None;
                }
                let head = &path_seg[..path_seg.len() - tail.len()];
                if head.is_empty() {
                    return None;
                }
                params.push((name.to_string(), head.to_string()));
                continue;
            }
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
// A route ending in `:name?` declares two routes, the parameter present and
// absent, exactly as the pin expands `OPTIONAL_PARAM_REGEXP` does.
//
// A `(...)` source is a real regex over its segment, compiled by `Regex` below.
// A whole trailing `*` segment is the pin's wildcard, and `maxParamLength`
// bounds a capture so a wildcard sibling can take over.
//
// `caseSensitive: false` is the pin's default and this port's too: static
// parts compare ASCII-case-insensitively and the duplicate key is lowercased,
// while a `:param` capture keeps the request path's own case.
//
// Not carried from the pin: `::` and `%` escaping in static parts, a wildcard
// glued to text in its segment (`/foo*`), and the regex constructs `Regex`
// names as unsupported.

/// Router configuration (`Router.RouterConfig`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RouterConfig {
    /// Collapse a run of slashes into one (`/a//b` matches `/a/b`).
    pub ignore_duplicate_slashes: bool,
    /// Drop a trailing slash (`/a/` matches `/a`).
    pub ignore_trailing_slash: bool,
    /// Longest a captured parameter may be (`maxParamLength`): a longer capture
    /// fails that route, which is how a wildcard sibling takes over.
    pub max_param_length: usize,
    /// Compare static parts ASCII-case-insensitively (`caseSensitive`). The
    /// pin defaults to `false`, so this port does too.
    pub case_sensitive: bool,
}

impl Default for RouterConfig {
    /// The pin's own defaults: both slash options on, a parameter capped at
    /// 100 characters, and case-insensitive matching.
    fn default() -> Self {
        Self {
            ignore_duplicate_slashes: true,
            ignore_trailing_slash: true,
            max_param_length: 100,
            case_sensitive: false,
        }
    }
}

/// Why a route registration was refused (`Router.on`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RouteError {
    /// Two routes under one method normalize to the same node path; the pin's
    /// message is `Method 'GET' already declared for route '/foo/:'`, so the
    /// key itself is carried as the payload.
    Duplicate { method: String, pattern: String },
    /// An optional parameter with something behind it (`Optional Parameter
    /// needs to be the last parameter of the path`).
    OptionalNotLast { pattern: String },
    /// A wildcard that is not the pattern's last segment (`Wildcard must be
    /// the last character in the route`).
    WildcardNotLast { pattern: String },
    /// The pattern uses a feature this slice does not carry: a wildcard glued
    /// to text in its segment, or a regex construct `Regex` refuses.
    Unsupported { pattern: String },
    /// A `(` in the pattern has no matching `)`, or a regex source that does
    /// not parse (`Invalid regexp expression`).
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
#[derive(Clone, Debug)]
enum SegmentPart {
    /// Literal text, matched byte for byte.
    Literal(String),
    /// A captured parameter. A capture may be empty: the pin slices the
    /// segment out of the path with no length check, which is what makes an
    /// optional parameter capture `""` at a trailing slash.
    Capture {
        /// Parameter name.
        name: String,
        /// The compiled `(...)` source, when the parameter has one.
        regex: Option<Regex>,
        /// What this capture refuses to contain. The pin compiles the first
        /// parameter of a segment to `(.*?)` and a later one to
        /// `(sep|(?:(?!sep).)*)`, so the separator before it stays out of the
        /// capture: `:p1-:p2` splits `foo-bar-baz` into `foo-bar` and `baz`.
        separator: Option<String>,
    },
    /// A whole `*` segment: it takes the rest of the path, slashes included.
    Wildcard,
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
    /// Normalized route key the duplicate check compares, lowercased when
    /// matching is case-insensitive (the pin's `route.pattern`).
    key: String,
    segments: Vec<Vec<SegmentPart>>,
    /// The pin's `isRegex` for the node this pattern creates: a regex source or
    /// any static part after a parameter makes the node a regex node.
    is_regex: bool,
    /// Whether the pattern ends in a wildcard segment, which the pin's walk
    /// tries after every parametric child.
    wildcard: bool,
    /// Length of the pattern's trailing static ending (the pin's `staticSuffix`).
    static_suffix_len: usize,
    handler: A,
}

impl<A> Pattern<A> {
    fn parse(
        method: &str,
        path: &str,
        handler: A,
        case_sensitive: bool,
    ) -> Result<Self, RouteError> {
        let mut key = String::new();
        let mut segments = Vec::new();
        let mut is_regex = false;
        let mut wildcard = false;
        let mut static_suffix_len = 0;
        let total = path.split('/').count();
        for (index, segment) in path.split('/').enumerate() {
            if index > 0 {
                key.push('/');
            }
            // A wildcard is a whole segment, and only the last one: a `*`
            // inside a parameter's source is that source's quantifier.
            if segment == "*" {
                if index + 1 != total {
                    return Err(RouteError::WildcardNotLast {
                        pattern: path.to_owned(),
                    });
                }
                key.push('*');
                wildcard = true;
                segments.push(vec![SegmentPart::Wildcard]);
                continue;
            }
            if has_bare_wildcard(segment) {
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
            // The pin's `isParamSafe` flag: a regex source resets it, the first
            // plain parameter of a segment is lazy, and every later one is
            // tempered by the static part just before it.
            let mut safe = true;
            let mut separator = String::new();
            for group in groups {
                let compiled = match group.regex.as_deref() {
                    Some(source) => Some(Regex::compile(source).map_err(|error| match error {
                        RegexError::Malformed => RouteError::Invalid {
                            pattern: path.to_owned(),
                        },
                        RegexError::Unsupported => RouteError::Unsupported {
                            pattern: path.to_owned(),
                        },
                    })?),
                    None => None,
                };
                let tempered = if group.regex.is_some() {
                    safe = true;
                    None
                } else {
                    let tempered = if safe { None } else { Some(separator.clone()) };
                    safe = false;
                    tempered
                };
                parts.push(SegmentPart::Capture {
                    name: group.name,
                    regex: compiled,
                    separator: tempered,
                });
                if !group.trailing.is_empty() {
                    separator = group.trailing.clone();
                    parts.push(SegmentPart::Literal(group.trailing));
                }
            }
            segments.push(parts);
        }
        // The pin lowercases `route.pattern` before its duplicate check, so two
        // routes differing only in case are one route here too.
        let key = if case_sensitive {
            key
        } else {
            key.to_ascii_lowercase()
        };
        Ok(Self {
            method: method.to_owned(),
            key,
            segments,
            is_regex,
            wildcard,
            static_suffix_len,
            handler,
        })
    }

    /// Match this route against an already-split path.
    fn match_segments(
        &self,
        path_segments: &[&str],
        max_param_length: usize,
        case_sensitive: bool,
    ) -> Option<Vec<(String, String)>> {
        let mut params = Vec::new();
        for (position, parts) in self.segments.iter().enumerate() {
            // A trailing wildcard takes the rest of the path, slashes included,
            // and needs a segment to start at (the pin reaches it only after
            // the slash that precedes it).
            if parts.len() == 1 && matches!(parts[0], SegmentPart::Wildcard) {
                path_segments.get(position)?;
                params.push(("*".to_owned(), path_segments[position..].join("/")));
                return Some(params);
            }
            params.extend(match_parts(
                parts,
                path_segments.get(position)?,
                max_param_length,
                case_sensitive,
            )?);
        }
        if self.segments.len() != path_segments.len() {
            return None;
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

/// Whether a segment is pure literal text (the pin's static child node).
fn is_static_segment(parts: &[SegmentPart]) -> bool {
    parts
        .iter()
        .all(|part| matches!(part, SegmentPart::Literal(_)))
}

/// Whether a segment carries a `*` outside a parameter's regex source (only the
/// whole-segment wildcard is carried).
fn has_bare_wildcard(segment: &str) -> bool {
    let bytes = segment.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => index += 2,
            b'(' => match closing_paren(segment, index) {
                Some(end) => index = end + 1,
                None => return false,
            },
            b'*' => return true,
            _ => index += 1,
        }
    }
    false
}

/// Route order for `find`: static segments first (the pin's walk tries a static
/// child before descending into a parametric one), then regex nodes before
/// plain parameters, then the longer static ending.
fn compare_routes<A>(left: &Pattern<A>, right: &Pattern<A>) -> std::cmp::Ordering {
    for (left_segment, right_segment) in left.segments.iter().zip(right.segments.iter()) {
        let left_static = is_static_segment(left_segment);
        let right_static = is_static_segment(right_segment);
        if left_static != right_static {
            return if left_static {
                std::cmp::Ordering::Less
            } else {
                std::cmp::Ordering::Greater
            };
        }
    }
    // A wildcard is the last child the pin's walk tries, after every
    // parametric one.
    (
        !left.is_regex,
        usize::MAX - left.static_suffix_len,
        left.wildcard,
    )
        .cmp(&(
            !right.is_regex,
            usize::MAX - right.static_suffix_len,
            right.wildcard,
        ))
}

/// Match one segment's parts against one path segment, returning the captures
/// in pattern order. A capture longer than `max_param_length` fails the route,
/// as the pin's `maxParamLength` does.
fn match_parts(
    parts: &[SegmentPart],
    rest: &str,
    max_param_length: usize,
    case_sensitive: bool,
) -> Option<Vec<(String, String)>> {
    let Some((first, tail)) = parts.split_first() else {
        return rest.is_empty().then(Vec::new);
    };
    match first {
        SegmentPart::Literal(text) => {
            let remainder = if case_sensitive {
                rest.strip_prefix(text.as_str())
            } else {
                // `get` rather than a slice: an index that is not a character
                // boundary is not a match either.
                rest.get(..text.len())
                    .filter(|head| head.eq_ignore_ascii_case(text))
                    .map(|_| &rest[text.len()..])
            };
            match_parts(tail, remainder?, max_param_length, case_sensitive)
        }
        // Only `match_segments` places a wildcard, in the pattern's last segment.
        SegmentPart::Wildcard => None,
        SegmentPart::Capture {
            name,
            regex,
            separator,
        } => {
            let mut ends: Vec<usize> = Vec::new();
            match regex {
                // The source is a real regex: every length it can match from
                // here, in the engine's own order (a greedy repeat first).
                Some(source) => ends.extend(source.ends(rest, 0)),
                None => match separator {
                    // Lazy: shortest capture first, the pin's `(.*?)`.
                    None => ends.extend(
                        rest.char_indices()
                            .map(|(index, _)| index)
                            .chain([rest.len()]),
                    ),
                    // Tempered: the separator itself, then the longest run free
                    // of it, giving a character back at a time.
                    Some(separator) => {
                        if rest.starts_with(separator.as_str()) {
                            ends.push(separator.len());
                        }
                        let limit = rest.find(separator.as_str()).unwrap_or(rest.len());
                        let mut boundaries: Vec<usize> = rest
                            .char_indices()
                            .map(|(index, _)| index)
                            .take_while(|index| *index <= limit)
                            .collect();
                        if boundaries.last() != Some(&limit) {
                            boundaries.push(limit);
                        }
                        ends.extend(boundaries.into_iter().rev());
                    }
                },
            }
            for end in ends {
                if end > max_param_length {
                    continue;
                }
                if let Some(mut params) =
                    match_parts(tail, &rest[end..], max_param_length, case_sensitive)
                {
                    let mut out = vec![(name.clone(), rest[..end].to_owned())];
                    out.append(&mut params);
                    return Some(out);
                }
            }
            None
        }
    }
}

/// Split a path whose last parameter is optional (`:name?`) into the path with
/// the parameter and the path without it (`OPTIONAL_PARAM_REGEXP` + the pin's
/// two replacements).
///
/// The pin's regexp is `(\/:[^/()]*?)\?(\/?)`: the parameter begins at a `:`
/// directly after a slash, never spans a slash or a parenthesis, and one slash
/// after the `?` belongs to the shorter form. The match has to reach the end of
/// the path, which is the pin's assertion.
fn optional_split(path: &str) -> Result<Option<(String, String)>, ()> {
    let bytes = path.as_bytes();
    let mut index = 0;
    while index + 1 < bytes.len() {
        if bytes[index] != b'/' || bytes[index + 1] != b':' {
            index += 1;
            continue;
        }
        let mut cursor = index + 2;
        let mut question = None;
        while cursor < bytes.len() && !matches!(bytes[cursor], b'/' | b'(' | b')') {
            if bytes[cursor] == b'?' {
                question = Some(cursor);
                break;
            }
            cursor += 1;
        }
        let Some(question) = question else {
            index += 2;
            continue;
        };
        let mut after = question + 1;
        if bytes.get(after) == Some(&b'/') {
            after += 1;
        }
        if after != bytes.len() {
            return Err(());
        }
        let full = format!("{}{}", &path[..question], &path[question + 1..]);
        // The shorter form keeps the slash that followed the `?` (group 2),
        // and an empty result is the root, as the pin's `|| "/"` says.
        let without = format!("{}{}", &path[..index], &path[question + 1..]);
        let without = if without.is_empty() {
            "/".to_owned()
        } else {
            without
        };
        return Ok(Some((full, without)));
    }
    Ok(None)
}

/// Apply the router's slash options to a route pattern or a request path
/// (`removeDuplicateSlashes` then `trimLastSlash`, in the pin's order).
fn normalize_slashes(path: &str, config: &RouterConfig) -> String {
    let mut text = path.to_owned();
    if config.ignore_duplicate_slashes && text.contains("//") {
        let mut collapsed = String::with_capacity(text.len());
        let mut previous_slash = false;
        for character in text.chars() {
            if character == '/' {
                if previous_slash {
                    continue;
                }
                previous_slash = true;
            } else {
                previous_slash = false;
            }
            collapsed.push(character);
        }
        text = collapsed;
    }
    if config.ignore_trailing_slash && text.len() > 1 && text.ends_with('/') {
        text.pop();
    }
    text
}

// -- Route-parameter regex sources ----------------------------------------
//
// The pin hands each `(...)` source to `new RegExp("^" + source + "$")` and
// reads the text the source matched. This is the subset a route pattern needs:
// literals, `.`, the class escapes (`\d \D \w \W \s \S`), escaped punctuation,
// groups, alternation, the `* + ?` quantifiers, `^`/`$`, and the `(?= )` /
// `(?! )` lookaheads. A construct outside it is refused when the route is
// registered (`[`, `{`, a backreference, a lookbehind), never read as a
// literal, so a pattern this port cannot honor is a loud error and not a route
// that quietly matches the wrong paths.
//
// ponytail: backtracking with no memoization, so a source like `(a*)*` can
// explore deeply. Route patterns are written by the app rather than the
// request, and the carried rows stay linear; a memoized scan is the upgrade
// path if a hostile pattern ever matters.

/// A compiled route-parameter source.
#[derive(Clone, Debug)]
struct Regex {
    /// Top-level alternatives, in source order.
    alternatives: Vec<Vec<Piece>>,
}

/// Why a source could not be compiled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RegexError {
    /// The source is malformed: an unclosed group, or a quantifier with
    /// nothing to repeat.
    Malformed,
    /// The source uses a construct this slice does not carry.
    Unsupported,
}

/// One unit of a compiled source.
#[derive(Clone, Debug)]
enum Piece {
    /// One character, matched exactly.
    Char(char),
    /// `.`: any character but a line break, which no request path holds.
    Any,
    /// A class from `\d`, `\D`, `\w`, `\W`, `\s`, or `\S`.
    Class(Class),
    /// A `(...)` group, matching what its alternatives match.
    Group(Box<Regex>),
    /// A `(?= )` or `(?! )` lookahead: zero width.
    Look {
        ahead: Box<Regex>,
        negative: bool,
    },
    /// `^` and `$` inside a source.
    AnchorStart,
    AnchorEnd,
    /// A quantified piece.
    Repeat {
        piece: Box<Piece>,
        min: usize,
        max: Option<usize>,
        greedy: bool,
    },
}

/// The character classes a route pattern can name (`\d` and friends).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Class {
    Digit,
    NotDigit,
    Word,
    NotWord,
    Space,
    NotSpace,
}

impl Class {
    /// Whether `character` is in the class. `\w` follows JavaScript's ASCII
    /// word rule rather than Rust's Unicode one.
    fn matches(self, character: char) -> bool {
        let digit = character.is_ascii_digit();
        let word = character.is_ascii_alphanumeric() || character == '_';
        let space = character.is_whitespace();
        match self {
            Class::Digit => digit,
            Class::NotDigit => !digit,
            Class::Word => word,
            Class::NotWord => !word,
            Class::Space => space,
            Class::NotSpace => !space,
        }
    }
}

impl Regex {
    /// Compile one `(...)` source: the text between the pin's parentheses,
    /// with its edge anchors already trimmed.
    fn compile(source: &str) -> Result<Self, RegexError> {
        let mut cursor = Source {
            chars: source.chars().collect(),
            at: 0,
        };
        let compiled = cursor.alternatives()?;
        if cursor.at != cursor.chars.len() {
            // A `)` with no `(` before it.
            return Err(RegexError::Malformed);
        }
        Ok(compiled)
    }

    /// The end of every match this source can make in `text` from `start`, in
    /// the engine's preference order: a greedy repeat offers its longest first.
    fn ends(&self, text: &str, start: usize) -> Vec<usize> {
        let mut out = Vec::new();
        for alternative in &self.alternatives {
            out.extend(match_sequence(alternative, text, start));
        }
        out
    }
}

/// A cursor over one source's characters.
struct Source {
    chars: Vec<char>,
    at: usize,
}

impl Source {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.at).copied()
    }

    fn alternatives(&mut self) -> Result<Regex, RegexError> {
        let mut out = vec![self.sequence()?];
        while self.peek() == Some('|') {
            self.at += 1;
            out.push(self.sequence()?);
        }
        Ok(Regex { alternatives: out })
    }

    fn sequence(&mut self) -> Result<Vec<Piece>, RegexError> {
        let mut out = Vec::new();
        loop {
            match self.peek() {
                None | Some('|') | Some(')') => return Ok(out),
                _ => out.push(self.piece()?),
            }
        }
    }

    fn piece(&mut self) -> Result<Piece, RegexError> {
        let atom = self.atom()?;
        let (min, max) = match self.peek() {
            Some('*') => {
                self.at += 1;
                (0, None)
            }
            Some('+') => {
                self.at += 1;
                (1, None)
            }
            Some('?') => {
                self.at += 1;
                (0, Some(1))
            }
            _ => return Ok(atom),
        };
        // A trailing `?` makes the quantifier lazy, and a lazy repeat offers
        // its shortest match first.
        let greedy = if self.peek() == Some('?') {
            self.at += 1;
            false
        } else {
            true
        };
        Ok(Piece::Repeat {
            piece: Box::new(atom),
            min,
            max,
            greedy,
        })
    }

    fn atom(&mut self) -> Result<Piece, RegexError> {
        let character = self.peek().ok_or(RegexError::Malformed)?;
        self.at += 1;
        Ok(match character {
            '.' => Piece::Any,
            '^' => Piece::AnchorStart,
            '$' => Piece::AnchorEnd,
            '(' => {
                let mut negative = false;
                let mut look = false;
                if self.peek() == Some('?') {
                    self.at += 1;
                    match self.peek() {
                        Some('=') => look = true,
                        Some('!') => {
                            look = true;
                            negative = true;
                        }
                        // `(?:` is the plain sequence; flags, group names, and
                        // lookbehind are not carried.
                        Some(':') => {}
                        _ => return Err(RegexError::Unsupported),
                    }
                    self.at += 1;
                }
                let inner = self.alternatives()?;
                if self.peek() != Some(')') {
                    return Err(RegexError::Malformed);
                }
                self.at += 1;
                if look {
                    Piece::Look {
                        ahead: Box::new(inner),
                        negative,
                    }
                } else {
                    Piece::Group(Box::new(inner))
                }
            }
            ')' => return Err(RegexError::Malformed),
            '\\' => self.escape()?,
            // A character class and a repetition count are outside the carried
            // subset, as is a quantifier with nothing to repeat.
            '[' | '{' => return Err(RegexError::Unsupported),
            '*' | '+' | '?' => return Err(RegexError::Malformed),
            other => Piece::Char(other),
        })
    }

    fn escape(&mut self) -> Result<Piece, RegexError> {
        let character = self.peek().ok_or(RegexError::Malformed)?;
        self.at += 1;
        Ok(match character {
            'd' => Piece::Class(Class::Digit),
            'D' => Piece::Class(Class::NotDigit),
            'w' => Piece::Class(Class::Word),
            'W' => Piece::Class(Class::NotWord),
            's' => Piece::Class(Class::Space),
            'S' => Piece::Class(Class::NotSpace),
            'n' => Piece::Char('\n'),
            'r' => Piece::Char('\r'),
            't' => Piece::Char('\t'),
            // A backreference, a word boundary, a unicode escape, or a property
            // escape: all outside the carried subset.
            '0'..='9' | 'b' | 'B' | 'u' | 'x' | 'p' | 'P' | 'k' => {
                return Err(RegexError::Unsupported);
            }
            // Anything else is the literal character (`\.`, `\-`, `\/`).
            other => Piece::Char(other),
        })
    }
}

/// The ends a sequence of pieces can reach from `start`, in preference order.
fn match_sequence(pieces: &[Piece], text: &str, start: usize) -> Vec<usize> {
    match pieces.split_first() {
        None => vec![start],
        Some((first, rest)) => {
            let mut out = Vec::new();
            for end in match_piece(first, text, start) {
                out.extend(match_sequence(rest, text, end));
            }
            out
        }
    }
}

/// The ends one piece can reach from `start`, in preference order.
fn match_piece(piece: &Piece, text: &str, start: usize) -> Vec<usize> {
    let next = text[start..].chars().next();
    match piece {
        Piece::Char(expected) => match next {
            Some(found) if found == *expected => vec![start + found.len_utf8()],
            _ => Vec::new(),
        },
        Piece::Any => match next {
            Some(found) if found != '\n' && found != '\r' => vec![start + found.len_utf8()],
            _ => Vec::new(),
        },
        Piece::Class(class) => match next {
            Some(found) if class.matches(found) => vec![start + found.len_utf8()],
            _ => Vec::new(),
        },
        Piece::Group(inner) => inner.ends(text, start),
        Piece::Look { ahead, negative } => {
            let matched = !ahead.ends(text, start).is_empty();
            if matched == *negative {
                Vec::new()
            } else {
                vec![start]
            }
        }
        Piece::AnchorStart => {
            if start == 0 {
                vec![start]
            } else {
                Vec::new()
            }
        }
        Piece::AnchorEnd => {
            if start == text.len() {
                vec![start]
            } else {
                Vec::new()
            }
        }
        Piece::Repeat {
            piece,
            min,
            max,
            greedy,
        } => repeat_ends(piece, text, start, *min, *max, *greedy),
    }
}

/// The ends a quantified piece can reach from `start`. A piece that matches
/// nothing cannot extend the frontier, which is what keeps an unbounded repeat
/// terminating.
fn repeat_ends(
    piece: &Piece,
    text: &str,
    start: usize,
    min: usize,
    max: Option<usize>,
    greedy: bool,
) -> Vec<usize> {
    let mut ends: Vec<usize> = Vec::new();
    let mut frontier = vec![start];
    let mut repeats = 0usize;
    loop {
        if repeats >= min {
            ends.extend(frontier.iter().copied());
        }
        if Some(repeats) == max {
            break;
        }
        let mut next: Vec<usize> = Vec::new();
        for position in &frontier {
            for end in match_piece(piece, text, *position) {
                if end > *position && !next.contains(&end) {
                    next.push(end);
                }
            }
        }
        if next.is_empty() {
            break;
        }
        next.sort_unstable();
        frontier = next;
        repeats += 1;
    }
    ends.sort_unstable();
    ends.dedup();
    if greedy {
        ends.reverse();
    }
    ends
}

/// A pattern router (`FindMyWay`): registers routes by method and pattern, and
/// resolves a request path to the highest-priority match.
#[derive(Clone, Debug)]
pub struct Router<A> {
    config: RouterConfig,
    routes: Vec<Pattern<A>>,
}

impl<A: Clone> Router<A> {
    /// An empty router with both slash options off (`Router.make()`).
    pub fn make() -> Self {
        Self::make_with(RouterConfig::default())
    }

    /// An empty router with explicit options (`Router.make(config)`).
    pub fn make_with(config: RouterConfig) -> Self {
        Self {
            config,
            routes: Vec::new(),
        }
    }

    /// Register one route (`Router.on`). The pin throws here; the refusal is a
    /// value so the caller decides. A trailing `:name?` registers the path with
    /// and without that parameter.
    pub fn on(&mut self, method: &str, path: &str, handler: A) -> Result<(), RouteError> {
        let mut patterns = Vec::new();
        match optional_split(path) {
            Err(()) => {
                return Err(RouteError::OptionalNotLast {
                    pattern: path.to_owned(),
                });
            }
            Ok(Some((full, without))) => {
                for path in [full, without] {
                    patterns.push(Pattern::parse(
                        method,
                        &normalize_slashes(&path, &self.config),
                        handler.clone(),
                        self.config.case_sensitive,
                    )?);
                }
            }
            Ok(None) => patterns.push(Pattern::parse(
                method,
                &normalize_slashes(path, &self.config),
                handler,
                self.config.case_sensitive,
            )?),
        }
        // Both halves of an optional declaration are checked before either is
        // registered, so a refused declaration leaves the router unchanged (the
        // pin throws mid-way and keeps the first half).
        for (index, pattern) in patterns.iter().enumerate() {
            let clash = self
                .routes
                .iter()
                .chain(patterns[..index].iter())
                .any(|route| route.method == pattern.method && route.key == pattern.key);
            if clash {
                return Err(RouteError::Duplicate {
                    method: pattern.method.clone(),
                    pattern: pattern.key.clone(),
                });
            }
        }
        self.routes.extend(patterns);
        Ok(())
    }

    /// Highest-priority route matching `method` and `path`, with its captures
    /// (`Router.find`). The query, fragment, and `;`-separated session id are
    /// stripped before matching, as the pin's `safeDecodeURI` does. Static
    /// parts fold case when the config says so, but a capture keeps the case
    /// the request itself used (the pin matches a lowercased path and slices
    /// its captures out of `originPath`).
    pub fn find(&self, method: &str, path: &str) -> Option<Found<A>> {
        let path = path.split(['?', '#', ';']).next().unwrap_or(path);
        let path = normalize_slashes(path, &self.config);
        let segments: Vec<&str> = path.split('/').collect();
        let mut candidates: Vec<&Pattern<A>> = self
            .routes
            .iter()
            .filter(|route| route.method == method)
            .collect();
        // Static segments first, then regex nodes, then the longest static
        // ending; the sort is stable, so equally ranked routes keep
        // registration order, which is where the pin's own comparator gives up
        // too.
        candidates.sort_by(|left, right| compare_routes(left, right));
        candidates.into_iter().find_map(|route| {
            route
                .match_segments(
                    &segments,
                    self.config.max_param_length,
                    self.config.case_sensitive,
                )
                .map(|params| Found {
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

/// Normalize a router prefix: leading slash ensured, trailing slashes
/// stripped, root `/` folds to empty (no prefix).
pub fn normalize_prefix(prefix: &str) -> String {
    let mut out = String::from(prefix);
    if !out.starts_with('/') {
        out.insert(0, '/');
    }
    while out.len() > 1 && out.ends_with('/') {
        out.pop();
    }
    if out == "/" {
        out.clear();
    }
    out
}

/// One route with its prefix attached (`HttpRouter.prefixRoute`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrefixedRoute {
    /// Uppercase method (`"GET"`).
    pub method: String,
    /// Local path (`"/users"`).
    pub path: String,
    /// Normalized prefix (`"/api"`), if any.
    pub prefix: Option<String>,
}

/// Attach `prefix` to one route (`HttpRouter.prefixRoute`).
pub fn prefix_route(method: &str, path: &str, prefix: &str) -> PrefixedRoute {
    PrefixedRoute {
        method: method.to_owned(),
        path: path.to_owned(),
        prefix: Some(normalize_prefix(prefix)),
    }
}

/// A prefix-scoped router: routes registered under a scope dispatch on the
/// full path but handlers see the prefix-stripped local URL (`HttpRouter`
/// with `prefixed`, `addAll`, and nested `prefixRoute` mounts).
pub struct PrefixRouter {
    scope: String,
    routes: Vec<PrefixedRouteEntry>,
}

struct PrefixedRouteEntry {
    method: String,
    prefix: String,
    path: String,
    handler: fn(&str) -> Response,
}

impl PrefixRouter {
    /// An empty router with no scope.
    pub fn new() -> Self {
        Self {
            scope: String::new(),
            routes: Vec::new(),
        }
    }

    /// A view registering under `prefix` (nesting concatenates scopes).
    pub fn prefixed(mut self, prefix: &str) -> Self {
        self.scope = normalize_prefix(prefix);
        self
    }

    /// Register `path` under this router's scope.
    pub fn add(&mut self, method: &str, path: &str, handler: fn(&str) -> Response) {
        self.routes.push(PrefixedRouteEntry {
            method: method.to_owned(),
            prefix: self.scope.clone(),
            path: path.to_owned(),
            handler,
        });
    }

    /// Register `path` under an explicit extra `prefix` beneath this scope.
    pub fn add_prefixed(
        &mut self,
        method: &str,
        path: &str,
        prefix: &str,
        handler: fn(&str) -> Response,
    ) {
        let prefix = format!("{}{}", self.scope, normalize_prefix(prefix));
        self.routes.push(PrefixedRouteEntry {
            method: method.to_owned(),
            prefix,
            path: path.to_owned(),
            handler,
        });
    }

    /// Mount a child router: its entries keep their local paths and gain the
    /// parent scope ahead of their own prefix.
    pub fn mount(&mut self, child: PrefixRouter) {
        for entry in child.routes {
            let prefix = format!("{}{}", self.scope, entry.prefix);
            self.routes.push(PrefixedRouteEntry { prefix, ..entry });
        }
    }

    /// Dispatch `method` + full `path`: the first entry whose prefix + path
    /// matches wins, and the handler receives the prefix-stripped local URL.
    pub fn dispatch(&self, method: &str, path: &str) -> Option<Response> {
        self.routes.iter().find_map(|entry| {
            if entry.method != method {
                return None;
            }
            let full = format!("{}{}", entry.prefix, entry.path);
            (full == path).then(|| (entry.handler)(&entry.path))
        })
    }
}

impl Default for PrefixRouter {
    fn default() -> Self {
        Self::new()
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
    if path.is_empty() { "/" } else { path }
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
                    response.status,
                    request.method,
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

/// CORS policy: the allowed origins, or allow-all when absent
/// (`HttpMiddleware.cors` with and without `allowedOrigins`).
#[derive(Debug, Clone, Default)]
pub struct CorsOptions {
    allowed_origins: Option<Vec<String>>,
}

impl CorsOptions {
    /// Allow exactly these origins.
    pub fn allow_origins(origins: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            allowed_origins: Some(origins.into_iter().map(Into::into).collect()),
        }
    }

    /// Allow every origin (`*`).
    pub fn allow_all() -> Self {
        Self {
            allowed_origins: None,
        }
    }
}

/// The CORS-relevant slice of a request.
#[derive(Debug, Clone)]
pub struct CorsRequest {
    /// Request `Origin`, if any.
    pub origin: Option<String>,
    /// Whether this is a preflight (`OPTIONS` with `Access-Control-Request-*`).
    pub is_preflight: bool,
}

/// Apply the CORS policy: the `Access-Control-Allow-Origin` value and the
/// resulting `Vary` value given the response's current `Vary`.
///
/// An allowed origin is echoed; a rejected one yields no allow-origin but
/// still varies on `Origin`. A wildcard `Vary` is never rewritten, and an
/// already-listed `Origin` (any case) is never duplicated. Preflight
/// requests additionally vary on `Access-Control-Request-Headers`.
/// Allow-all echoes `*` and leaves `Vary` untouched.
pub fn apply_cors(
    options: &CorsOptions,
    request: &CorsRequest,
    response_vary: Option<&str>,
) -> (Option<String>, Option<String>) {
    if options.allowed_origins.is_none() {
        return (Some("*".to_owned()), response_vary.map(str::to_owned));
    }
    let allowed = request.origin.as_ref().is_some_and(|origin| {
        options
            .allowed_origins
            .as_ref()
            .is_some_and(|list| list.iter().any(|item| item == origin))
    });
    let allow_origin = if allowed {
        request.origin.clone()
    } else {
        None
    };
    let mut vary = response_vary.map(str::to_owned);
    vary = Some(ensure_vary_dimension(vary.as_deref(), "Origin"));
    if request.is_preflight {
        vary = Some(ensure_vary_dimension(
            vary.as_deref(),
            "Access-Control-Request-Headers",
        ));
    }
    (allow_origin, vary)
}

/// `Vary` with `dimension` present exactly once (case-insensitive); `*` wins.
fn ensure_vary_dimension(vary: Option<&str>, dimension: &str) -> String {
    match vary {
        None | Some("") => dimension.to_owned(),
        Some("*") => "*".to_owned(),
        Some(current) => {
            if current
                .split(',')
                .any(|member| member.trim().eq_ignore_ascii_case(dimension))
            {
                current.to_owned()
            } else {
                format!("{current}, {dimension}")
            }
        }
    }
}

/// Logger annotations for one request/response pair: method, path without
/// query or fragment, and numeric status (`HttpMiddleware.logger`).
pub fn log_annotations(method: &str, url: &str, status: u16) -> Vec<(String, String)> {
    vec![
        ("http.method".to_owned(), method.to_owned()),
        ("http.url".to_owned(), request_path(url).to_owned()),
        ("http.status".to_owned(), status.to_string()),
    ]
}

/// The path of `url`, dropping query and fragment; `/` when absent.
fn request_path(url: &str) -> &str {
    let after_authority = match url.split_once("://") {
        Some((_, rest)) => rest.split_at(rest.find('/').unwrap_or(rest.len())).1,
        None => url,
    };
    let path = after_authority
        .split(['?', '#'])
        .next()
        .unwrap_or(after_authority);
    if path.is_empty() { "/" } else { path }
}

/// JavaScript's `Number.MAX_SAFE_INTEGER`: the largest `content-length` the
/// pin keeps (`"9007199254740992"` is already unsafe and dropped).
const MAX_SAFE_CONTENT_LENGTH: u64 = 9_007_199_254_740_991;

/// Parsed server-side request head: the `HttpServerRequest.fromWeb` /
/// `toClientRequest` / `fromClientRequest` kernel that needs no live server
/// or web runtime (method, URL pieces, headers, buffered body, remote
/// address). A buffered body is re-readable by every accessor, which is what
/// the pin's shared concurrent stream accessors observe; stream sharing and
/// schema decoding stay on the Effect side.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerRequestHead {
    /// The request verb (`"POST"`, …).
    pub method: String,
    /// The full request URL, the pin's `source`/`originalUrl`.
    pub original_url: String,
    /// Request headers.
    pub headers: Vec<(String, String)>,
    /// Buffered body bytes; empty means no body (the pin's `Empty` tag).
    pub body: Vec<u8>,
    /// Remote address; web requests default to none.
    pub remote_address: Option<String>,
}

impl ServerRequestHead {
    /// A head with no headers, no body, and no remote address.
    pub fn new(method: &str, url: &str) -> Self {
        Self {
            method: method.to_owned(),
            original_url: url.to_owned(),
            headers: Vec::new(),
            body: Vec::new(),
            remote_address: None,
        }
    }

    /// First header value for `name` (case-insensitive), if present.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    /// The client URL: scheme, authority, and path, without query or
    /// fragment (`toClientRequest` moves those onto `urlParams`/`hash`).
    pub fn client_url(&self) -> &str {
        let without_fragment = self
            .original_url
            .split('#')
            .next()
            .unwrap_or(&self.original_url);
        without_fragment
            .split('?')
            .next()
            .unwrap_or(without_fragment)
    }

    /// The server URL: path plus query plus fragment
    /// (`"/items?existing=1&page=1#top"`); `/` when absent.
    pub fn server_url(&self) -> &str {
        let after_authority = match self.original_url.split_once("://") {
            Some((_, rest)) => rest.split_at(rest.find('/').unwrap_or(rest.len())).1,
            None => self.original_url.as_str(),
        };
        if after_authority.is_empty() {
            "/"
        } else {
            after_authority
        }
    }

    /// The fragment without `#`, if the URL carries one.
    pub fn fragment(&self) -> Option<&str> {
        self.original_url.split_once('#').map(|(_, hash)| hash)
    }

    /// Query pairs in order, duplicates kept (`urlParams`); raw, without
    /// percent-decoding (that rides the router's `decode_component`).
    pub fn query_params(&self) -> Vec<(String, String)> {
        let without_fragment = self
            .original_url
            .split('#')
            .next()
            .unwrap_or(&self.original_url);
        let query = match without_fragment.split_once('?') {
            Some((_, query)) => query,
            None => return Vec::new(),
        };
        query
            .split('&')
            .filter(|pair| !pair.is_empty())
            .map(|pair| match pair.split_once('=') {
                Some((name, value)) => (name.to_owned(), value.to_owned()),
                None => (pair.to_owned(), String::new()),
            })
            .collect()
    }

    /// Cookies parsed from every `Cookie` header (`name=value` pairs split
    /// on `;`), in order.
    pub fn cookies(&self) -> Vec<(String, String)> {
        self.headers
            .iter()
            .filter(|(name, _)| name.eq_ignore_ascii_case("cookie"))
            .flat_map(|(_, value)| value.split(';'))
            .filter_map(|pair| {
                let (name, value) = pair.split_once('=')?;
                Some((name.trim().to_owned(), value.trim().to_owned()))
            })
            .collect()
    }

    /// First cookie value for `name`, if present.
    pub fn cookie(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .filter(|(header, _)| header.eq_ignore_ascii_case("cookie"))
            .flat_map(|(_, value)| value.split(';'))
            .filter_map(|pair| {
                let (key, value) = pair.split_once('=')?;
                (key.trim() == name).then_some(value.trim())
            })
            .next()
    }

    /// True when there is no body (the pin's `Empty` tag).
    pub fn is_empty_body(&self) -> bool {
        self.body.is_empty()
    }

    /// The `content-length` when it is well-formed: non-empty ASCII digits
    /// within the safe-integer range. Malformed or unsafe values
    /// (`"2junk"`, `"1.5"`, `"1e3"`, past `MAX_SAFE_INTEGER`) yield none
    /// while the body itself stays buffered.
    pub fn sanitized_content_length(&self) -> Option<u64> {
        let raw = self.header("content-length")?;
        if raw.is_empty() || !raw.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        let length: u64 = raw.parse().ok()?;
        (length <= MAX_SAFE_CONTENT_LENGTH).then_some(length)
    }

    /// The body as text.
    pub fn text_body(&self) -> Result<String, String> {
        String::from_utf8(self.body.clone()).map_err(|error| error.to_string())
    }

    /// The body parsed as JSON.
    pub fn json_body(&self) -> Result<serde_json::Value, String> {
        serde_json::from_slice(&self.body).map_err(|error| error.to_string())
    }

    /// Set the remote address (`modify` with an explicit value).
    pub fn set_remote_address(&mut self, address: &str) {
        self.remote_address = Some(address.to_owned());
    }

    /// Clear the remote address (`modify` with an explicit none); doing
    /// nothing keeps it.
    pub fn clear_remote_address(&mut self) {
        self.remote_address = None;
    }
}
