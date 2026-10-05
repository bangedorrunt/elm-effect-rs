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
    /// Optional URL fragment (without `#`).
    pub hash: Option<String>,
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
            hash: None,
        }
    }

    /// A POST request with a body.
    pub fn post(url: &str, body: &str) -> HttpRequest {
        HttpRequest {
            method: HttpMethod::POST,
            url: url.to_string(),
            headers: Vec::new(),
            body: Some(body.as_bytes().to_vec()),
            hash: None,
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

    /// Remove every header with `name` (case-insensitive); true when any
    /// was removed. Missing names are a no-op returning false.
    pub fn remove_header(&mut self, name: &str) -> bool {
        let before = self.headers.len();
        self.headers.retain(|(n, _)| !n.eq_ignore_ascii_case(name));
        self.headers.len() != before
    }

    /// Set the URL fragment (without `#`).
    pub fn set_hash(&mut self, hash: impl Into<String>) {
        self.hash = Some(hash.into());
    }

    /// Clear the URL fragment.
    pub fn remove_hash(&mut self) {
        self.hash = None;
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

    /// Remove every name in `names`; returns the count of names that
    /// removed something.
    pub fn remove_many(&mut self, names: &[&str]) -> usize {
        names.iter().filter(|name| self.remove(name)).count()
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
    /// Expires date, if present.
    pub expires: Option<String>,
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
    /// An already-expired cookie for `name` (clears it in the client).
    /// Renders with `Max-Age=0` plus a past `Expires` date.
    pub fn expired(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            value: String::new(),
            max_age: Some(0),
            expires: Some("Thu, 01 Jan 1970 00:00:00 GMT".to_string()),
            domain: None,
            path: Some("/".to_string()),
            secure: false,
            http_only: false,
            same_site: None,
        }
    }

    /// True for RFC 6265 token names (no separators, spaces, or controls).
    pub fn is_valid_name(name: &str) -> bool {
        !name.is_empty() && name.bytes().all(|b| {
            b.is_ascii_alphanumeric()
                || matches!(b, b'!' | b'#' | b'$' | b'%' | b'&' | b'\'' | b'*' | b'+' | b'-' | b'.' | b'^' | b'_' | b'`' | b'|' | b'~')
        })
    }

    /// Parse one `name=value` pair (no attributes). Invalid names fail.
    pub fn parse_pair(pair: &str) -> Option<Self> {
        let (name, value) = pair.split_once('=')?;
        let name = name.trim();
        if !Self::is_valid_name(name) {
            return None;
        }
        Some(Self {
            name: name.to_string(),
            value: Self::unquote(value.trim()),
            max_age: None,
            expires: None,
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
                ("expires", Some(v)) => cookie.expires = Some(v.to_string()),
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
        if let Some(expires) = &self.expires {
            out.push_str(&format!("; Expires={expires}"));
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

/// An ordered jar of request cookies.
///
/// Ports the `Cookies` collection core: header parsing (invalid names
/// dropped), `get`/`get_value` returning `Option`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Cookies {
    cookies: Vec<Cookie>,
}

impl Cookies {
    /// Empty jar.
    pub fn new() -> Self {
        Self::default()
    }

    /// Parse a `Cookie` header, dropping invalid names.
    pub fn parse_header(header: &str) -> Self {
        Self {
            cookies: Cookie::parse_header(header),
        }
    }

    /// Build from name/value pairs, dropping invalid names.
    pub fn from_pairs(pairs: Vec<(String, String)>) -> Self {
        Self {
            cookies: pairs
                .into_iter()
                .filter(|(name, _)| Cookie::is_valid_name(name))
                .map(|(name, value)| Cookie {
                    name,
                    value,
                    max_age: None,
                    expires: None,
                    domain: None,
                    path: None,
                    secure: false,
                    http_only: false,
                    same_site: None,
                })
                .collect(),
        }
    }

    /// First cookie with `name`.
    pub fn get(&self, name: &str) -> Option<&Cookie> {
        self.cookies.iter().find(|c| c.name == name)
    }

    /// First cookie value with `name`.
    pub fn get_value(&self, name: &str) -> Option<&str> {
        self.get(name).map(|c| c.value.as_str())
    }

    /// True when no cookies are held.
    pub fn is_empty(&self) -> bool {
        self.cookies.is_empty()
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

/// A parsed absolute URL with mutation.
///
/// Ports the portable core of `Url.ts` (`fromString`, setters, relative
/// resolution): scheme, authority (optional userinfo + host + port), path,
/// query (kept raw; use [`UrlParams`] for structure), and fragment.
/// Rendering preserves the parts; passwords render as stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Url {
    /// `https` in `https://…` (lowercased).
    pub scheme: String,
    /// Optional `user` (never includes the password).
    pub username: String,
    /// Optional password.
    pub password: Option<String>,
    /// Host without brackets (lowercased).
    pub host: String,
    /// Explicit port, if any.
    pub port: Option<u16>,
    /// Path starting with `/` (defaults to `/`).
    pub path: String,
    /// Raw query without `?`.
    pub query: String,
    /// Fragment without `#`.
    pub fragment: String,
}

impl Url {
    /// Parse an absolute URL. Fails on missing/invalid scheme, missing
    /// host, bad port, or whitespace.
    pub fn parse(text: &str) -> Option<Self> {
        if text.bytes().any(|b| b.is_ascii_whitespace()) {
            return None;
        }
        let (scheme, rest) = text.split_once("://")?;
        if scheme.is_empty() || !scheme.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'.')) {
            return None;
        }
        let (authority, path_query_frag) = match rest.find('/') {
            Some(i) => (&rest[..i], &rest[i..]),
            None => (rest, "/"),
        };
        if authority.is_empty() {
            return None;
        }
        let (userinfo, host_port) = match authority.rsplit_once('@') {
            Some((u, h)) => (Some(u), h),
            None => (None, authority),
        };
        let (username, password) = match userinfo {
            Some(u) => match u.split_once(':') {
                Some((name, pass)) => (name.to_string(), Some(pass.to_string())),
                None => (u.to_string(), None),
            },
            None => (String::new(), None),
        };
        let (host, port) = if let Some(stripped) = host_port.strip_prefix('[') {
            let (host, rest) = stripped.split_once(']')?;
            let port = match rest.strip_prefix(':') {
                Some(p) if !p.is_empty() => Some(p.parse().ok()?),
                Some(_) => return None,
                None if rest.is_empty() => None,
                None => return None,
            };
            (format!("[{host}]"), port)
        } else {
            match host_port.rsplit_once(':') {
                Some((h, p)) if !h.is_empty() => {
                    let port: u16 = if p.is_empty() { return None } else { p.parse().ok()? };
                    (h.to_ascii_lowercase(), Some(port))
                }
                _ => (host_port.to_ascii_lowercase(), None),
            }
        };
        if host.is_empty() {
            return None;
        }
        let (path_query, fragment) = match path_query_frag.split_once('#') {
            Some((pq, f)) => (pq, f.to_string()),
            None => (path_query_frag, String::new()),
        };
        let (path, query) = match path_query.split_once('?') {
            Some((p, q)) => (p.to_string(), q.to_string()),
            None => (path_query.to_string(), String::new()),
        };
        Some(Self {
            scheme: scheme.to_ascii_lowercase(),
            username,
            password,
            host,
            port,
            path: if path.is_empty() { "/".to_string() } else { path },
            query,
            fragment,
        })
    }

    /// Resolve a relative reference against this base URL.
    /// Absolute references parse directly; absolute paths keep the base
    /// authority; other references merge onto the base directory.
    pub fn resolve(&self, reference: &str) -> Option<Self> {
        if reference.contains("://") {
            return Self::parse(reference);
        }
        if let Some(path) = reference.strip_prefix('/') {
            let mut resolved = self.clone();
            let (path, query_frag) = match path.find(['?', '#']) {
                Some(i) => (&path[..i], &path[i..]),
                None => (path, ""),
            };
            resolved.path = format!("/{path}");
            resolved.query = query_frag.strip_prefix('?').unwrap_or(query_frag).split('#').next().unwrap_or("").to_string();
            resolved.fragment = query_frag.split('#').nth(1).unwrap_or("").to_string();
            return Some(resolved);
        }
        let mut base_dir = self.path.clone();
        if let Some(i) = base_dir.rfind('/') {
            base_dir.truncate(i + 1);
        }
        let (ref_path, ref_query_frag) = match reference.find(['?', '#']) {
            Some(i) => (&reference[..i], &reference[i..]),
            None => (reference, ""),
        };
        let merged = normalize_path(&format!("{base_dir}{ref_path}"));
        let mut resolved = self.clone();
        resolved.path = merged;
        resolved.query = ref_query_frag
            .strip_prefix('?')
            .unwrap_or(ref_query_frag)
            .split('#')
            .next()
            .unwrap_or("")
            .to_string();
        resolved.fragment = ref_query_frag.split('#').nth(1).unwrap_or("").to_string();
        Some(resolved)
    }

    /// Render the full URL.
    pub fn render(&self) -> String {
        let mut out = format!("{}://", self.scheme);
        if !self.username.is_empty() {
            out.push_str(&self.username);
            if let Some(password) = &self.password {
                out.push(':');
                out.push_str(password);
            }
            out.push('@');
        }
        out.push_str(&self.host);
        if let Some(port) = self.port {
            out.push_str(&format!(":{port}"));
        }
        out.push_str(&self.path);
        if !self.query.is_empty() {
            out.push('?');
            out.push_str(&self.query);
        }
        if !self.fragment.is_empty() {
            out.push('#');
            out.push_str(&self.fragment);
        }
        out
    }

    /// Structured query parameters.
    pub fn url_params(&self) -> UrlParams {
        UrlParams::parse(&self.query)
    }

    /// Replace the query from structured parameters.
    pub fn set_url_params(&mut self, params: &UrlParams) {
        self.query = params.render();
    }
}

fn normalize_path(path: &str) -> String {
    let mut segments: Vec<&str> = Vec::new();
    for segment in path.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                segments.pop();
            }
            other => segments.push(other),
        }
    }
    format!("/{}", segments.join("/"))
}

impl Url {
    /// Replace the fragment (`#` added on render).
    pub fn set_hash(&mut self, hash: impl Into<String>) {
        let hash = hash.into();
        self.fragment = hash.strip_prefix('#').unwrap_or(&hash).to_string();
    }

    /// Replace host, accepting `host` or `host:port`.
    pub fn set_host(&mut self, host: impl Into<String>) {
        let host = host.into();
        if let Some(stripped) = host.strip_prefix('[') {
            if let Some((h, rest)) = stripped.split_once(']') {
                self.host = format!("[{h}]");
                self.port = rest.strip_prefix(':').and_then(|p| p.parse().ok());
                return;
            }
        }
        match host.rsplit_once(':') {
            Some((h, p)) if !h.is_empty() => {
                if let Ok(port) = p.parse::<u16>() {
                    self.host = h.to_ascii_lowercase();
                    self.port = Some(port);
                    return;
                }
                self.host = host.to_ascii_lowercase();
                self.port = None;
            }
            _ => {
                self.host = host.to_ascii_lowercase();
                self.port = None;
            }
        }
    }

    /// Replace the hostname, keeping the port.
    pub fn set_hostname(&mut self, hostname: impl Into<String>) {
        self.host = hostname.into().to_ascii_lowercase();
    }

    /// Replace the whole URL by re-parsing; keeps the old value when the
    /// replacement does not parse.
    pub fn set_href(&mut self, href: &str) {
        if let Some(url) = Self::parse(href) {
            *self = url;
        }
    }

    /// Replace the password.
    pub fn set_password(&mut self, password: impl Into<String>) {
        self.password = Some(password.into());
    }

    /// Replace the path (a leading `/` is added when missing).
    pub fn set_pathname(&mut self, pathname: impl Into<String>) {
        let pathname = pathname.into();
        self.path = if pathname.starts_with('/') {
            pathname
        } else {
            format!("/{pathname}")
        };
    }

    /// Replace the port (`None` clears it).
    pub fn set_port(&mut self, port: Option<u16>) {
        self.port = port;
    }

    /// Replace the scheme (lowercased).
    pub fn set_protocol(&mut self, protocol: impl Into<String>) {
        let protocol = protocol.into();
        let scheme = protocol.strip_suffix(':').unwrap_or(&protocol);
        self.scheme = scheme.to_ascii_lowercase();
    }

    /// Replace the query (a leading `?` is stripped).
    pub fn set_search(&mut self, search: impl Into<String>) {
        let search = search.into();
        self.query = search.strip_prefix('?').unwrap_or(&search).to_string();
    }

    /// Replace the username.
    pub fn set_username(&mut self, username: impl Into<String>) {
        self.username = username.into();
    }

    /// Apply several mutations at once, returning the updated URL.
    pub fn mutate(mut self, f: impl FnOnce(&mut Self)) -> Self {
        f(&mut self);
        self
    }

    /// Render with the password masked.
    pub fn redacted(&self) -> String {
        let mut masked = self.clone();
        if masked.password.is_some() {
            masked.password = Some("***".to_string());
        }
        masked.render()
    }
}

/// Join a base URL and a path with exactly one slash between them.
///
/// Ports `HttpClientRequest` path joining: empty paths no-op, existing
/// and leading slashes are preserved without doubling.
pub fn join_url(base: &str, path: &str) -> String {
    if path.is_empty() {
        return base.to_string();
    }
    let base = base.strip_suffix('/').unwrap_or(base);
    if path.starts_with('/') {
        format!("{base}{path}")
    } else {
        format!("{base}/{path}")
    }
}
