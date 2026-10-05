//! HTTP client service, API routes, and network addresses.
//!
//! The value layer (requests, routes, addresses) is always available.
//! Actually sending bytes needs feature `http` (ureq 2); without it
//! [`HttpClient::send`] fails with [`HttpError::Disabled`].

/// An HTTP method: the verbs we route and send.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HttpMethod {
    GET,
    POST,
    PUT,
    DELETE,
    PATCH,
    HEAD,
    OPTIONS,
}

/// A built request: method, URL, headers, optional body bytes.
#[derive(Debug, Clone, PartialEq)]
pub struct HttpRequest {
    /// The request verb.
    pub method: HttpMethod,
    /// The full URL.
    pub url: String,
    /// Header pairs in insertion order.
    pub headers: Vec<(String, String)>,
    /// The optional body.
    pub body: Option<Vec<u8>>,
}

/// A received response: status, headers, body bytes.
#[derive(Debug, Clone, PartialEq)]
pub struct HttpResponse {
    /// The status code.
    pub status: u16,
    /// Header pairs in arrival order.
    pub headers: Vec<(String, String)>,
    /// The body bytes.
    pub body: Vec<u8>,
}

impl HttpResponse {
    /// The body as UTF-8 text, if valid.
    pub fn text(&self) -> Option<&str> {
        std::str::from_utf8(&self.body).ok()
    }
}

/// What can go wrong sending a request.
#[derive(Debug, Clone, PartialEq)]
pub enum HttpError {
    /// Feature `http` is off: no transport compiled in.
    Disabled,
    /// The URL or request is malformed.
    InvalidRequest(String),
    /// The transport failed.
    Transport(String),
    /// A non-2xx status the caller treats as failure.
    BadStatus(u16),
}

impl std::fmt::Display for HttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HttpError::Disabled => write!(f, "http transport disabled (feature http off)"),
            HttpError::InvalidRequest(message) => write!(f, "invalid request: {message}"),
            HttpError::Transport(message) => write!(f, "transport error: {message}"),
            HttpError::BadStatus(status) => write!(f, "bad status: {status}"),
        }
    }
}

impl std::error::Error for HttpError {}

/// The client: builds requests; sends them when the transport exists.
pub struct HttpClient;

impl HttpClient {
    /// A GET request with no body.
    pub fn get(url: &str) -> HttpRequest {
        HttpRequest {
            method: HttpMethod::GET,
            url: url.to_string(),
            headers: Vec::new(),
            body: None,
        }
    }

    /// A POST request with a body.
    pub fn post(url: &str, body: &str) -> HttpRequest {
        HttpRequest {
            method: HttpMethod::POST,
            url: url.to_string(),
            headers: Vec::new(),
            body: Some(body.as_bytes().to_vec()),
        }
    }

    /// Send a request synchronously (ureq 2 when compiled in).
    pub fn send(request: &HttpRequest) -> Result<HttpResponse, HttpError> {
        #[cfg(feature = "http")]
        return Self::send_ureq(request);
        #[cfg(not(feature = "http"))]
        {
            let _ = request;
            Err(HttpError::Disabled)
        }
    }

    #[cfg(feature = "http")]
    fn send_ureq(request: &HttpRequest) -> Result<HttpResponse, HttpError> {
        let verb = match request.method {
            HttpMethod::GET => "GET",
            HttpMethod::POST => "POST",
            HttpMethod::PUT => "PUT",
            HttpMethod::DELETE => "DELETE",
            HttpMethod::PATCH => "PATCH",
            HttpMethod::HEAD => "HEAD",
            HttpMethod::OPTIONS => "OPTIONS",
        };
        let mut outgoing = ureq::request(verb, &request.url);
        for (name, value) in &request.headers {
            outgoing = outgoing.set(name, value);
        }
        let response = match &request.body {
            Some(body) => outgoing.send_bytes(body),
            None => outgoing.call(),
        }
        .map_err(|error| HttpError::Transport(error.to_string()))?;
        let status = response.status();
        let mut headers = Vec::new();
        for name in response.headers_names() {
            if let Some(value) = response.header(&name) {
                headers.push((name, value.to_string()));
            }
        }
        let mut body = Vec::new();
        response
            .into_reader()
            .read_to_end(&mut body)
            .map_err(|error| HttpError::Transport(error.to_string()))?;
        Ok(HttpResponse {
            status,
            headers,
            body,
        })
    }
}

impl HttpRequest {
    /// Attach `name: value`, preserving order.
    pub fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_string(), value.to_string()));
        self
    }
}

/// A static route table: method + path patterns to handler names.
/// `:segment` patterns match one path segment each.
pub struct Api {
    routes: Vec<(HttpMethod, Vec<String>, &'static str)>,
}

impl Api {
    /// An empty table.
    pub fn new() -> Self {
        Self { routes: Vec::new() }
    }

    /// Register `pattern` (`:segment` matches one level) for `method`.
    pub fn route(mut self, method: HttpMethod, pattern: &str, handler: &'static str) -> Self {
        self.routes.push((
            method,
            pattern.split('/').map(str::to_string).collect(),
            handler,
        ));
        self
    }

    /// The handler for `method` + `path`, if any pattern matches.
    pub fn lookup(&self, method: &HttpMethod, path: &str) -> Option<&'static str> {
        let segments: Vec<&str> = path.split('/').collect();
        self.routes.iter().find_map(|(verb, pattern, handler)| {
            if verb != method || pattern.len() != segments.len() {
                return None;
            }
            let matches = pattern
                .iter()
                .zip(segments.iter())
                .all(|(rule, actual)| rule.starts_with(':') || rule == actual);
            matches.then_some(*handler)
        })
    }
}

impl Default for Api {
    fn default() -> Self {
        Self::new()
    }
}

/// A parsed `host:port` network address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetAddr {
    /// The host part.
    pub host: String,
    /// The port part.
    pub port: u16,
}

impl NetAddr {
    /// Split `host:port`; both parts must be present and valid.
    pub fn parse(text: &str) -> Result<Self, String> {
        let (host, port) = text
            .rsplit_once(':')
            .ok_or_else(|| format!("missing :port in {text:?}"))?;
        if host.is_empty() {
            return Err(format!("missing host in {text:?}"));
        }
        let port: u16 = port
            .parse()
            .map_err(|_| format!("invalid port in {text:?}"))?;
        Ok(Self {
            host: host.to_string(),
            port,
        })
    }

    /// Well-known local address kinds.
    pub fn localhost(port: u16) -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port,
        }
    }
}

/// Case-insensitive multi-value header map.
///
/// Ports the portable core of `packages/effect/src/http/Headers.ts`:
/// names compare ASCII-case-insensitively, `get` returns the last set
/// value, `get_all` every value in insertion order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Headers {
    entries: Vec<(String, String)>,
}

impl Headers {
    /// Empty headers.
    pub fn new() -> Self {
        Self::default()
    }

    /// Build from pairs.
    pub fn from_pairs(pairs: Vec<(String, String)>) -> Self {
        let mut headers = Self::new();
        for (name, value) in pairs {
            headers.set(name, value);
        }
        headers
    }

    /// Set (replacing every existing value for `name`).
    pub fn set(&mut self, name: impl Into<String>, value: impl Into<String>) {
        let name = name.into();
        self.entries.retain(|(n, _)| !n.eq_ignore_ascii_case(&name));
        self.entries.push((name, value.into()));
    }

    /// Append without removing existing values.
    pub fn append(&mut self, name: impl Into<String>, value: impl Into<String>) {
        self.entries.push((name.into(), value.into()));
    }

    /// Last value for `name`, case-insensitively.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.entries
            .iter()
            .rev()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    /// Every value for `name` in insertion order.
    pub fn get_all(&self, name: &str) -> Vec<&str> {
        self.entries
            .iter()
            .filter(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
            .collect()
    }

    /// True when `name` is present.
    pub fn has(&self, name: &str) -> bool {
        self.get(name).is_some()
    }

    /// Remove every value for `name`; true when something was removed.
    pub fn remove(&mut self, name: &str) -> bool {
        let before = self.entries.len();
        self.entries.retain(|(n, _)| !n.eq_ignore_ascii_case(name));
        self.entries.len() != before
    }

    /// Merge another map in (set semantics per name).
    pub fn merge(&mut self, other: &Headers) {
        for (name, value) in &other.entries {
            self.set(name.clone(), value.clone());
        }
    }

    /// All entries in insertion order.
    pub fn entries(&self) -> &[(String, String)] {
        &self.entries
    }
}

/// One parsed `Cookie` request pair plus optional `Set-Cookie` attributes.
///
/// Ports the portable core of `Cookies.ts`: `Cookie` header parsing and
/// `Set-Cookie` serialization (Expires/Max-Age/Domain/Path/Secure/HttpOnly/
/// SameSite). `__Secure-`/`__Host-` prefix rules are not enforced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cookie {
    /// Cookie name.
    pub name: String,
    /// Cookie value (decoded, without surrounding quotes).
    pub value: String,
    /// Max-Age seconds, if present.
    pub max_age: Option<u64>,
    /// Domain attribute, if present.
    pub domain: Option<String>,
    /// Path attribute, if present.
    pub path: Option<String>,
    /// Secure attribute.
    pub secure: bool,
    /// HttpOnly attribute.
    pub http_only: bool,
    /// SameSite attribute (`Strict`, `Lax`, or `None`).
    pub same_site: Option<String>,
}

impl Cookie {
    /// Parse one `name=value` pair (no attributes).
    pub fn parse_pair(pair: &str) -> Option<Self> {
        let (name, value) = pair.split_once('=')?;
        Some(Self {
            name: name.trim().to_string(),
            value: Self::unquote(value.trim()),
            max_age: None,
            domain: None,
            path: None,
            secure: false,
            http_only: false,
            same_site: None,
        })
    }

    /// Parse a full `Cookie` request header into pairs.
    pub fn parse_header(header: &str) -> Vec<Self> {
        header.split(';').filter_map(Self::parse_pair).collect()
    }

    /// Parse a `Set-Cookie` response value (first segment is the pair,
    /// the rest are attributes).
    pub fn parse_set_cookie(header: &str) -> Option<Self> {
        let mut segments = header.split(';');
        let mut cookie = Self::parse_pair(segments.next()?.trim())?;
        for segment in segments {
            let segment = segment.trim();
            let (attr, val) = match segment.split_once('=') {
                Some((a, v)) => (a.trim().to_ascii_lowercase(), Some(v.trim())),
                None => (segment.to_ascii_lowercase(), None),
            };
            match (attr.as_str(), val) {
                ("max-age", Some(v)) => cookie.max_age = v.parse().ok(),
                ("domain", Some(v)) => cookie.domain = Some(v.to_string()),
                ("path", Some(v)) => cookie.path = Some(v.to_string()),
                ("secure", _) => cookie.secure = true,
                ("httponly", _) => cookie.http_only = true,
                ("samesite", Some(v)) => cookie.same_site = Some(v.to_string()),
                _ => {}
            }
        }
        Some(cookie)
    }

    /// Serialize as a `Set-Cookie` value.
    pub fn to_set_cookie(&self) -> String {
        let mut out = format!("{}={}", self.name, self.value);
        if let Some(max_age) = self.max_age {
            out.push_str(&format!("; Max-Age={max_age}"));
        }
        if let Some(domain) = &self.domain {
            out.push_str(&format!("; Domain={domain}"));
        }
        if let Some(path) = &self.path {
            out.push_str(&format!("; Path={path}"));
        }
        if self.secure {
            out.push_str("; Secure");
        }
        if self.http_only {
            out.push_str("; HttpOnly");
        }
        if let Some(same_site) = &self.same_site {
            out.push_str(&format!("; SameSite={same_site}"));
        }
        out
    }

    fn unquote(value: &str) -> String {
        if value.len() >= 2 && value.starts_with('"') && value.ends_with('"') {
            value[1..value.len() - 1].to_string()
        } else {
            value.to_string()
        }
    }
}

/// An entity tag for cache validation.
///
/// Ports `Etag.ts`: strong vs weak tags, `to_string` rendering, and the
/// `If-None-Match` freshness check. Tag generation uses FNV-1a over the
/// body (the reference plugs in a hasher; the algorithm is transport).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Etag {
    /// Byte-exact match required.
    Strong(String),
    /// Semantically equivalent match suffices.
    Weak(String),
}

impl Etag {
    /// Generate a strong tag for `body`.
    pub fn strong_for(body: &[u8]) -> Self {
        Self::Strong(format!("{:x}", fnv1a(body)))
    }

    /// Generate a weak tag for `body`.
    pub fn weak_for(body: &[u8]) -> Self {
        Self::Weak(format!("{:x}", fnv1a(body)))
    }

    /// Render for an `ETag` header (`W/` prefix for weak tags).
    pub fn render(&self) -> String {
        match self {
            Self::Strong(tag) => format!("\"{tag}\""),
            Self::Weak(tag) => format!("W/\"{tag}\""),
        }
    }

    /// True when `if_none_match` (a comma list, possibly `*`) matches.
    /// A weak comparison is used, matching the reference semantics: tags
    /// match when their opaque values are equal regardless of strength.
    pub fn is_fresh(&self, if_none_match: &str) -> bool {
        let mine = self.opaque();
        if_none_match.split(',').any(|candidate| {
            let candidate = candidate.trim();
            candidate == "*" || strip_weak(candidate) == mine
        })
    }

    fn opaque(&self) -> &str {
        match self {
            Self::Strong(tag) | Self::Weak(tag) => tag,
        }
    }
}

fn strip_weak(tag: &str) -> &str {
    let tag = tag.strip_prefix("W/").unwrap_or(tag);
    tag.strip_prefix('"')
        .and_then(|t| t.strip_suffix('"'))
        .unwrap_or(tag)
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100_0000_01b3);
    }
    hash
}

/// A request/response body.
///
/// Ports the portable core of `HttpBody.ts` (Uint8Array/Raw/Empty/FormData;
/// `Stream` bodies need a runtime stream and `File` bodies need a file
/// system, so neither is modeled here).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HttpBody {
    /// No body.
    Empty,
    /// Raw bytes.
    Bytes(Vec<u8>),
    /// UTF-8 text.
    Text(String),
    /// URL-encoded form fields.
    Form(Vec<(String, String)>),
}

impl HttpBody {
    /// Content length in bytes when known without rendering.
    pub fn content_length(&self) -> Option<usize> {
        match self {
            Self::Empty => Some(0),
            Self::Bytes(bytes) => Some(bytes.len()),
            Self::Text(text) => Some(text.len()),
            Self::Form(_) => None,
        }
    }

    /// The `Content-Type` for this body, if it implies one.
    pub fn content_type(&self) -> Option<&'static str> {
        match self {
            Self::Empty | Self::Bytes(_) => None,
            Self::Text(_) => Some("text/plain; charset=utf-8"),
            Self::Form(_) => Some("application/x-www-form-urlencoded"),
        }
    }

    /// Render to bytes (form fields are percent-encoded pairs joined by `&`).
    pub fn render(&self) -> Vec<u8> {
        match self {
            Self::Empty => Vec::new(),
            Self::Bytes(bytes) => bytes.clone(),
            Self::Text(text) => text.as_bytes().to_vec(),
            Self::Form(fields) => fields
                .iter()
                .map(|(k, v)| format!("{}={}", percent_encode(k), percent_encode(v)))
                .collect::<Vec<_>>()
                .join("&")
                .into_bytes(),
        }
    }
}

fn percent_encode(text: &str) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(byte as char);
        } else if byte == b' ' {
            out.push('+');
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// Ordered URL query parameters with repeat support.
///
/// Ports the portable core of `UrlParams.ts`: pairs keep insertion order,
/// names may repeat, a missing `=` is a `None` value. `get_first`/`get_last`
/// return `Option` (`None` values read as empty strings).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UrlParams {
    pairs: Vec<(String, Option<String>)>,
}

impl UrlParams {
    /// Empty parameters.
    pub fn new() -> Self {
        Self::default()
    }

    /// Build from pairs.
    pub fn from_pairs(pairs: Vec<(String, Option<String>)>) -> Self {
        Self { pairs }
    }

    /// Parse a query string (leading `?` allowed).
    pub fn parse(query: &str) -> Self {
        let query = query.strip_prefix('?').unwrap_or(query);
        if query.is_empty() {
            return Self::new();
        }
        Self {
            pairs: query
                .split('&')
                .filter(|segment| !segment.is_empty())
                .map(|segment| match segment.split_once('=') {
                    Some((name, value)) => {
                        (percent_decode(name), Some(percent_decode(value)))
                    }
                    None => (percent_decode(segment), None),
                })
                .collect(),
        }
    }

    /// First value for `name`, or `None` when absent (`None` values read
    /// as empty strings).
    pub fn get_first(&self, name: &str) -> Option<&str> {
        self.pairs
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_deref().unwrap_or(""))
    }

    /// Last value for `name`, or `None` when absent.
    pub fn get_last(&self, name: &str) -> Option<&str> {
        self.pairs
            .iter()
            .rev()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_deref().unwrap_or(""))
    }

    /// Set, dropping every existing pair with `name` (overrides replace).
    pub fn set(&mut self, name: impl Into<String>, value: Option<String>) {
        let name = name.into();
        self.pairs.retain(|(n, _)| n != &name);
        self.pairs.push((name, value));
    }

    /// Append without dropping existing pairs.
    pub fn append(&mut self, name: impl Into<String>, value: Option<String>) {
        self.pairs.push((name.into(), value));
    }

    /// Remove every pair with `name`; true when something was removed.
    pub fn remove(&mut self, name: &str) -> bool {
        let before = self.pairs.len();
        self.pairs.retain(|(n, _)| n != name);
        self.pairs.len() != before
    }

    /// All pairs in insertion order.
    pub fn pairs(&self) -> &[(String, Option<String>)] {
        &self.pairs
    }

    /// Render back to a query string (`None` values render as bare keys).
    pub fn render(&self) -> String {
        self.pairs
            .iter()
            .map(|(name, value)| match value {
                Some(v) => format!("{}={}", percent_encode(name), percent_encode(v)),
                None => percent_encode(name),
            })
            .collect::<Vec<_>>()
            .join("&")
    }
}

fn hex_val(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn percent_decode(text: &str) -> String {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 <= bytes.len() => {
                match (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                    (Some(hi), Some(lo)) => {
                        out.push(hi << 4 | lo);
                        i += 3;
                    }
                    _ => {
                        out.push(b'%');
                        i += 1;
                    }
                }
            }
            byte => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}
