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

/// Numeric code for a status literal (`HttpStatus.fromLiteral`;
/// `None` for unknown names, including the `Ok` alias spelled `OK`).
pub fn http_status_from_literal(name: &str) -> Option<u16> {
    Some(match name {
        "Continue" => 100,
        "SwitchingProtocols" => 101,
        "Processing" => 102,
        "EarlyHints" => 103,
        "OK" => 200,
        "Ok" => 200,
        "Created" => 201,
        "Accepted" => 202,
        "NonAuthoritativeInformation" => 203,
        "NoContent" => 204,
        "ResetContent" => 205,
        "PartialContent" => 206,
        "MultiStatus" => 207,
        "AlreadyReported" => 208,
        "ImUsed" => 226,
        "MultipleChoices" => 300,
        "MovedPermanently" => 301,
        "Found" => 302,
        "SeeOther" => 303,
        "NotModified" => 304,
        "TemporaryRedirect" => 307,
        "PermanentRedirect" => 308,
        "BadRequest" => 400,
        "Unauthorized" => 401,
        "PaymentRequired" => 402,
        "Forbidden" => 403,
        "NotFound" => 404,
        "MethodNotAllowed" => 405,
        "NotAcceptable" => 406,
        "ProxyAuthenticationRequired" => 407,
        "RequestTimeout" => 408,
        "Conflict" => 409,
        "Gone" => 410,
        "LengthRequired" => 411,
        "PreconditionFailed" => 412,
        "PayloadTooLarge" => 413,
        "UriTooLong" => 414,
        "UnsupportedMediaType" => 415,
        "RangeNotSatisfiable" => 416,
        "ExpectationFailed" => 417,
        "ImATeapot" => 418,
        "MisdirectedRequest" => 421,
        "UnprocessableEntity" => 422,
        "Locked" => 423,
        "FailedDependency" => 424,
        "TooEarly" => 425,
        "UpgradeRequired" => 426,
        "PreconditionRequired" => 428,
        "TooManyRequests" => 429,
        "RequestHeaderFieldsTooLarge" => 431,
        "UnavailableForLegalReasons" => 451,
        "InternalServerError" => 500,
        "NotImplemented" => 501,
        "BadGateway" => 502,
        "ServiceUnavailable" => 503,
        "GatewayTimeout" => 504,
        "HttpVersionNotSupported" => 505,
        "VariantAlsoNegotiates" => 506,
        "InsufficientStorage" => 507,
        "LoopDetected" => 508,
        "NotExtended" => 510,
        "NetworkAuthenticationRequired" => 511,
        _ => return None,
    })
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

    /// Replace every header with `pairs` (`setHeaders`).
    pub fn set_headers(mut self, pairs: Vec<(String, String)>) -> Self {
        self.headers = pairs;
        self
    }

    /// Transform the whole header collection (`updateHeaders`).
    pub fn update_headers(mut self, f: impl FnOnce(Headers) -> Headers) -> Self {
        self.headers = f(Headers::from_pairs(self.headers)).entries().to_vec();
        self
    }

    /// The request as a parsed URL; `None` when the URL is malformed.
    pub fn to_url(&self) -> Option<Url> {
        Url::parse(&self.url)
    }

    /// The follow-up request for a redirect response, or `None` when
    /// this status/location pair is not a redirect.
    ///
    /// Ports `HttpClient.followRedirects` single-hop semantics from
    /// `packages/effect/src/http/HttpClient.ts`: 3xx with a location,
    /// 301/302 POST and non-GET/HEAD 303 switch to GET with the body
    /// dropped, and credential headers (`authorization`, `cookie`,
    /// `proxy-authorization`) are stripped when the origin (scheme,
    /// host, effective port) changes. The caller bounds the hop count.
    pub fn redirect_next(&self, status: u16, location: &str) -> Option<HttpRequest> {
        if !(300..400).contains(&status) || location.is_empty() {
            return None;
        }
        let base = Url::parse(&self.url)?;
        let target = base.resolve(location)?;
        let mut next = self.clone();
        if ((status == 301 || status == 302) && self.method == HttpMethod::POST)
            || (status == 303 && self.method != HttpMethod::GET && self.method != HttpMethod::HEAD)
        {
            next.method = HttpMethod::GET;
            next.body = None;
            next.headers.retain(|(name, _)| {
                !name.eq_ignore_ascii_case("content-length")
                    && !name.eq_ignore_ascii_case("content-type")
            });
        }
        if base.scheme != target.scheme
            || base.host != target.host
            || effective_port(&base) != effective_port(&target)
        {
            next.headers.retain(|(name, _)| {
                !name.eq_ignore_ascii_case("authorization")
                    && !name.eq_ignore_ascii_case("cookie")
                    && !name.eq_ignore_ascii_case("proxy-authorization")
            });
        }
        next.url = target.render();
        Some(next)
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

    /// A copy with every matching name's value replaced (case-insensitive
    /// name match). Non-matching entries pass through untouched.
    pub fn redacted(&self, names: &[&str], replacement: &str) -> Self {
        Self {
            entries: self
                .entries
                .iter()
                .map(|(name, value)| {
                    if names.iter().any(|n| n.eq_ignore_ascii_case(name)) {
                        (name.clone(), replacement.to_string())
                    } else {
                        (name.clone(), value.clone())
                    }
                })
                .collect(),
        }
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
        !name.is_empty()
            && name.bytes().all(|b| {
                b.is_ascii_alphanumeric()
                    || matches!(
                        b,
                        b'!' | b'#'
                            | b'$'
                            | b'%'
                            | b'&'
                            | b'\''
                            | b'*'
                            | b'+'
                            | b'-'
                            | b'.'
                            | b'^'
                            | b'_'
                            | b'`'
                            | b'|'
                            | b'~'
                    )
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

    /// Build from possibly-absent values, coercing absence to `"null"`
    /// (`UrlParams.fromInput` core for the null case; richer shapes
    /// arrive stringified at the call site).
    pub fn from_input(pairs: Vec<(String, Option<String>)>) -> Self {
        Self {
            pairs: pairs
                .into_iter()
                .map(|(name, value)| (name, Some(value.unwrap_or_else(|| "null".to_owned()))))
                .collect(),
        }
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
                    Some((name, value)) => (percent_decode(name), Some(percent_decode(value))),
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
            b'%' if i + 2 <= bytes.len() => match (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                (Some(hi), Some(lo)) => {
                    out.push(hi << 4 | lo);
                    i += 3;
                }
                _ => {
                    out.push(b'%');
                    i += 1;
                }
            },
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
        if scheme.is_empty()
            || !scheme
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'.'))
        {
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
                    let port: u16 = if p.is_empty() {
                        return None;
                    } else {
                        p.parse().ok()?
                    };
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
            path: if path.is_empty() {
                "/".to_string()
            } else {
                path
            },
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
            resolved.query = query_frag
                .strip_prefix('?')
                .unwrap_or(query_frag)
                .split('#')
                .next()
                .unwrap_or("")
                .to_string();
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

/// Build the request URL for an endpoint path template (`urlBuilder`).
///
/// `template` holds `:name` (required) and `:name?` (optional) placeholders;
/// only names present in `params` substitute, so an undeclared action suffix
/// such as the `:wait` in `/operations/:id:wait` stays literal. A segment
/// that is exactly a missing optional placeholder drops out (`/files/:path?`
/// with no `path` becomes `/files`). Values encode with `encodeURIComponent`
/// semantics (`/` becomes `%2F`). `query` renders in order, repeating names
/// for array values. `prefix` (the API prefix, `""` when none) joins ahead of
/// the template; `base` (`None` for relative URLs) joins ahead of that.
pub fn build_endpoint_url(
    base: Option<&str>,
    prefix: &str,
    template: &str,
    params: &[(&str, &str)],
    query: &[(String, Option<String>)],
) -> Result<String, String> {
    let joined = if prefix.is_empty() {
        template.to_string()
    } else {
        let prefix = prefix.strip_suffix('/').unwrap_or(prefix);
        if template.starts_with('/') {
            format!("{prefix}{template}")
        } else {
            format!("{prefix}/{template}")
        }
    };
    let mut kept = Vec::new();
    for segment in joined.split('/') {
        if segment.is_empty() {
            kept.push(String::new());
            continue;
        }
        if let Some(expanded) = expand_segment(segment, params) {
            kept.push(expanded);
        }
    }
    let path = kept.join("/");
    let path = if path.is_empty() {
        "/".to_string()
    } else {
        path
    };
    let mut url = match base {
        Some(base) => join_url(base, &path),
        None => path,
    };
    if !query.is_empty() {
        url.push('?');
        url.push_str(&UrlParams::from_pairs(query.to_vec()).render());
    }
    Ok(url)
}

/// Expand one path segment: substitute supplied params, drop a segment that
/// is only a missing optional placeholder, leave anything else literal.
fn expand_segment(segment: &str, params: &[(&str, &str)]) -> Option<String> {
    let chars: Vec<char> = segment.chars().collect();
    let mut index = 0;
    let mut out = String::new();
    let mut has_literal = false;
    while index < chars.len() {
        if chars[index] != ':' {
            out.push(chars[index]);
            has_literal = true;
            index += 1;
            continue;
        }
        let mut end = index + 1;
        while end < chars.len() && (chars[end].is_ascii_alphanumeric() || chars[end] == '_') {
            end += 1;
        }
        if end == index + 1 {
            out.push(':');
            has_literal = true;
            index += 1;
            continue;
        }
        let name: String = chars[index + 1..end].iter().collect();
        let mut optional = false;
        if end < chars.len() && chars[end] == '?' {
            optional = true;
            end += 1;
        }
        match params.iter().find(|(key, _)| *key == name) {
            Some((_, value)) => out.push_str(&encode_path_segment(value)),
            None if optional => {}
            None => {
                out.push(':');
                out.push_str(&name);
                if optional {
                    out.push('?');
                }
                has_literal = true;
            }
        }
        index = end;
    }
    if out.is_empty() && !has_literal {
        return None;
    }
    Some(out)
}

/// `encodeURIComponent`: unreserved marks pass through, everything else
/// (including `/` and space) becomes `%XX`.
fn encode_path_segment(value: &str) -> String {
    const UNRESERVED: &[u8] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_.!~*'()";
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        if UNRESERVED.contains(&byte) {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// Explicit port, else the scheme default (http 80, https 443).
/// Used for redirect origin comparison (`URL.origin` semantics).
fn effective_port(url: &Url) -> Option<u16> {
    url.port.or(match url.scheme.as_str() {
        "http" => Some(80),
        "https" => Some(443),
        _ => None,
    })
}

/// Annotation key for a display title (`OpenApi.Title` in TS).
pub const ANNOTATION_TITLE: &str = "title";
/// Annotation key for a description (`OpenApi.Description` in TS).
pub const ANNOTATION_DESCRIPTION: &str = "description";

/// Last value for `key` in an annotation list, or `None`.
fn annotation_get(entries: &[(String, String)], key: &str) -> Option<String> {
    entries
        .iter()
        .rev()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.clone())
}

/// `entries` with `key` set to `value` (replaces any existing value).
fn annotation_set(
    mut entries: Vec<(String, String)>,
    key: &str,
    value: &str,
) -> Vec<(String, String)> {
    entries.retain(|(k, _)| k != key);
    entries.push((key.to_string(), value.to_string()));
    entries
}

/// An endpoint in the API spec: identifier plus method and path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpApiEndpoint {
    /// The endpoint identifier.
    pub identifier: String,
    /// The endpoint verb.
    pub method: HttpMethod,
    /// The route path.
    pub path: String,
}

impl HttpApiEndpoint {
    /// A GET endpoint with `identifier` at `path`.
    pub fn get(identifier: &str, path: &str) -> Self {
        Self {
            identifier: identifier.to_string(),
            method: HttpMethod::GET,
            path: path.to_string(),
        }
    }
}

/// A group of endpoints with its own annotation scope.
///
/// Ports `HttpApiGroup`: identifiers are ordinary owned keys (a group
/// literally named `__proto__` needs no special case), and every
/// combinator returns a new value, leaving the source untouched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpApiGroup {
    /// The group identifier.
    pub identifier: String,
    /// Endpoints in insertion order.
    pub endpoints: Vec<(String, HttpApiEndpoint)>,
    /// Annotation scope, most specific last.
    pub annotations: Vec<(String, String)>,
    /// Top-level groups flatten their endpoints to the API root
    /// (`topLevel: true` in `HttpApiGroup.make`).
    pub top_level: bool,
}

impl HttpApiGroup {
    /// An empty group with `identifier`.
    pub fn make(identifier: &str) -> Self {
        Self {
            identifier: identifier.to_string(),
            endpoints: Vec::new(),
            annotations: Vec::new(),
            top_level: false,
        }
    }

    /// Add an endpoint, keyed by its identifier.
    pub fn add_endpoint(mut self, endpoint: HttpApiEndpoint) -> Self {
        self.endpoints.retain(|(id, _)| id != &endpoint.identifier);
        self.endpoints.push((endpoint.identifier.clone(), endpoint));
        self
    }

    /// The endpoint with `identifier`, if present.
    pub fn endpoint(&self, identifier: &str) -> Option<&HttpApiEndpoint> {
        self.endpoints
            .iter()
            .find(|(id, _)| id == identifier)
            .map(|(_, endpoint)| endpoint)
    }

    /// Set annotation `key` to `value`.
    pub fn annotate(self, key: &str, value: &str) -> Self {
        Self {
            annotations: annotation_set(self.annotations, key, value),
            ..self
        }
    }

    /// Flatten this group's endpoints to the API root in client builders
    /// (`topLevel: true`); the URL paths are unchanged, only the nesting is.
    pub fn top_level(mut self) -> Self {
        self.top_level = true;
        self
    }

    /// Merge a parent scope underneath this group's own annotations
    /// (the group's values win, mirroring `addHttpApi` precedence).
    pub fn annotate_merge(self, parent: &[(String, String)]) -> Self {
        let mut merged: Vec<(String, String)> = parent.to_vec();
        for (key, value) in &self.annotations {
            merged = annotation_set(merged, key, value);
        }
        Self {
            annotations: merged,
            ..self
        }
    }

    /// The annotation for `key`, if present.
    pub fn annotation(&self, key: &str) -> Option<&str> {
        self.annotations
            .iter()
            .rev()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }
}

/// An API spec: identifier, groups, and an annotation scope.
///
/// Ports `HttpApi.make`/`add`/`addHttpApi`/`annotate`/`reflect`: a data
/// value, so every combinator returns a new spec. `add_http_api`
/// flattens the other spec's own groups into fresh copies scoped with
/// its annotations, leaving both inputs unchanged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpApi {
    /// The API identifier.
    pub identifier: String,
    /// Groups in insertion order, keyed by identifier.
    pub groups: Vec<(String, HttpApiGroup)>,
    /// Annotation scope for groups added later via `add_http_api`.
    pub annotations: Vec<(String, String)>,
    /// Path prefix applied ahead of every endpoint path (`.prefix(...)`).
    pub prefix: String,
}

impl HttpApi {
    /// An empty spec with `identifier`.
    pub fn make(identifier: &str) -> Self {
        Self {
            identifier: identifier.to_string(),
            groups: Vec::new(),
            annotations: Vec::new(),
            prefix: String::new(),
        }
    }

    /// Apply a path prefix ahead of every endpoint path.
    pub fn prefix(mut self, scope: &str) -> Self {
        self.prefix = scope.to_string();
        self
    }

    /// Add a group, keyed by its identifier.
    pub fn add(mut self, group: HttpApiGroup) -> Self {
        self.groups.retain(|(id, _)| id != &group.identifier);
        self.groups.push((group.identifier.clone(), group));
        self
    }

    /// Flatten another spec's groups into this one. Each copied group
    /// merges the other spec's annotations underneath its own; groups
    /// already present here are untouched, as is the other spec.
    pub fn add_http_api(mut self, other: &HttpApi) -> Self {
        for (id, group) in &other.groups {
            let merged = group.clone().annotate_merge(&other.annotations);
            self.groups.retain(|(existing, _)| existing != id);
            self.groups.push((id.clone(), merged));
        }
        self
    }

    /// Set annotation `key` to `value` on this spec's scope.
    pub fn annotate(self, key: &str, value: &str) -> Self {
        Self {
            annotations: annotation_set(self.annotations, key, value),
            ..self
        }
    }

    /// The group with `identifier`, if present.
    pub fn group(&self, identifier: &str) -> Option<&HttpApiGroup> {
        self.groups
            .iter()
            .find(|(id, _)| id == identifier)
            .map(|(_, group)| group)
    }

    /// Own group identifiers in insertion order.
    pub fn group_keys(&self) -> Vec<&str> {
        self.groups.iter().map(|(id, _)| id.as_str()).collect()
    }

    /// The spec-scope annotation for `key`, if present.
    pub fn annotation(&self, key: &str) -> Option<&str> {
        self.annotations
            .iter()
            .rev()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    /// Walk every group then its endpoints in insertion order.
    pub fn reflect(
        &self,
        mut on_group: impl FnMut(&HttpApiGroup),
        mut on_endpoint: impl FnMut(&HttpApiGroup, &HttpApiEndpoint),
    ) {
        for (_, group) in &self.groups {
            on_group(group);
            for (_, endpoint) in &group.endpoints {
                on_endpoint(group, endpoint);
            }
        }
    }

    /// One tag per group: the title annotation names it, the merged
    /// description documents it (`OpenApi.fromApi` tag surface).
    pub fn tags(&self) -> Vec<ApiTag> {
        self.groups
            .iter()
            .map(|(_, group)| ApiTag {
                name: annotation_get(&group.annotations, ANNOTATION_TITLE)
                    .unwrap_or_else(|| group.identifier.clone()),
                description: annotation_get(&group.annotations, ANNOTATION_DESCRIPTION),
            })
            .collect()
    }
}

/// An OpenAPI-style tag derived from a group.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiTag {
    /// The tag name.
    pub name: String,
    /// The tag description, if annotated.
    pub description: Option<String>,
}

/// Why a multipart parse failed (TS `MultipartErrorReason`).
///
/// `status` renders 400 for `Parse`, 500 for `InternalError`, 413 for every
/// limit; every reason is ignored by the error reporter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MultipartReason {
    /// One file part exceeded the per-file limit.
    FileTooLarge,
    /// One field value exceeded the field-size limit.
    FieldTooLarge,
    /// The whole body exceeded the total-size limit.
    BodyTooLarge,
    /// More parts than allowed.
    TooManyParts,
    /// Transport or persistence failure carrying the body.
    InternalError,
    /// Malformed body or headers.
    Parse,
}

impl MultipartReason {
    /// Build the error for this reason.
    pub fn error(self) -> MultipartError {
        MultipartError::from_reason(self)
    }

    /// Response status for this reason.
    pub fn status(&self) -> u16 {
        multipart_status(self)
    }

    /// Every multipart error is ignored by the error reporter.
    pub fn is_ignored(&self) -> bool {
        true
    }

    /// The public message is the reason tag.
    pub fn message(&self) -> &'static str {
        match self {
            Self::FileTooLarge => "FileTooLarge",
            Self::FieldTooLarge => "FieldTooLarge",
            Self::BodyTooLarge => "BodyTooLarge",
            Self::TooManyParts => "TooManyParts",
            Self::InternalError => "InternalError",
            Self::Parse => "Parse",
        }
    }
}

/// A multipart parse failure (TS `MultipartError`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MultipartError {
    /// The concrete reason.
    pub reason: MultipartReason,
}

impl MultipartError {
    /// Build from a reason tag.
    pub fn from_reason(reason: MultipartReason) -> Self {
        Self { reason }
    }

    /// Response status for the reason.
    pub fn status(&self) -> u16 {
        multipart_status(&self.reason)
    }
}

impl std::fmt::Display for MultipartError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.reason.message())
    }
}

impl std::error::Error for MultipartError {}

/// Response status for a multipart reason: 400 parse, 500 internal, else 413.
pub fn multipart_status(reason: &MultipartReason) -> u16 {
    match reason {
        MultipartReason::Parse => 400,
        MultipartReason::InternalError => 500,
        MultipartReason::FileTooLarge
        | MultipartReason::FieldTooLarge
        | MultipartReason::BodyTooLarge
        | MultipartReason::TooManyParts => 413,
    }
}

/// True only for a real `MultipartError` (a plain tagged lookalike is false).
pub fn is_multipart_error(value: &dyn std::any::Any) -> bool {
    value.is::<MultipartError>()
}

/// One parsed multipart part: a decoded text field or a file's bytes.
///
/// Sync whole-body core of TS `Part`: streaming `content`/`contentEffect`
/// pull machinery has no runtime here, so files carry their bytes directly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MultipartPart {
    /// A decoded text field.
    Field {
        /// Field name.
        key: String,
        /// Part media type.
        content_type: String,
        /// Decoded value.
        value: String,
    },
    /// An uploaded file with its full bytes.
    File {
        /// Field name.
        key: String,
        /// Original file name (`filename`, else the field name).
        name: String,
        /// Part media type.
        content_type: String,
        /// File bytes.
        content: Vec<u8>,
    },
}

impl MultipartPart {
    /// Every part (field, file, persisted) is a part.
    pub fn is_part(&self) -> bool {
        true
    }

    /// Parsed field/file parts are stream parts; persisted files are not.
    pub fn is_stream_part(&self) -> bool {
        true
    }

    /// True for fields.
    pub fn is_field(&self) -> bool {
        matches!(self, Self::Field { .. })
    }

    /// True for files.
    pub fn is_file(&self) -> bool {
        matches!(self, Self::File { .. })
    }
}

/// A file written to a scoped temp dir (TS `PersistedFile`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistedFile {
    /// Field name.
    pub key: String,
    /// Original file name.
    pub name: String,
    /// Part media type.
    pub content_type: String,
    /// Scoped temp path.
    pub path: String,
}

impl PersistedFile {
    /// Persisted files are parts but not stream parts.
    pub fn is_part(&self) -> bool {
        true
    }

    /// Persisted files are no longer streamed.
    pub fn is_stream_part(&self) -> bool {
        false
    }

    /// Brand guard.
    pub fn is_persisted_file(&self) -> bool {
        true
    }
}

/// Parser limits (TS `MaxParts`/`MaxFieldSize`/`MaxFileSize`/`MaxBodySize`).
///
/// `max_file_size` is the per-file cap (`MaxPartSize` maps from `MaxFileSize`
/// in `makeConfig`). Field MIME types (default `application/json`) parse as
/// fields even with a filename.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MultipartLimits {
    /// Max part count (`None` = unbounded).
    pub max_parts: Option<usize>,
    /// Max field value bytes (default 10 MiB when `None`, matching TS).
    pub max_field_size: Option<usize>,
    /// Max file bytes per part (`None` = unbounded).
    pub max_file_size: Option<usize>,
    /// Max whole-body bytes (`None` = unbounded).
    pub max_total_size: Option<usize>,
    /// Content types parsed as fields even with a filename.
    pub field_mime_types: Vec<String>,
}

impl MultipartLimits {
    /// Default field cap is 10 MiB, matching the TS `MaxFieldSize` default.
    pub fn field_cap(&self) -> usize {
        self.max_field_size.unwrap_or(10 * 1024 * 1024)
    }

    /// MIME types treated as fields (default `application/json`).
    pub fn field_mimes(&self) -> Vec<String> {
        if self.field_mime_types.is_empty() {
            vec!["application/json".to_string()]
        } else {
            self.field_mime_types.clone()
        }
    }
}

/// The `boundary=` parameter of a `multipart/form-data` content type.
pub fn multipart_boundary(content_type: &str) -> Option<String> {
    for piece in content_type.split(';') {
        let piece = piece.trim();
        if let Some(rest) = piece
            .strip_prefix("boundary=")
            .or_else(|| piece.strip_prefix("BOUNDARY="))
        {
            let boundary = rest.trim().trim_matches('"');
            if !boundary.is_empty() {
                return Some(boundary.to_string());
            }
        }
    }
    None
}

/// Parse a whole multipart body (split chunks reassembled by the caller).
///
/// Ports the portable core of `Multipart.makeChannel`: boundary framing,
/// `Content-Disposition` names (UTF-8, plus `filename*` decoding),
/// field/file split, and every limit mapping (`TooManyParts`, `FieldTooLarge`,
/// `FileTooLarge`, `BodyTooLarge`, truncated bodies as `Parse`). Streaming
/// backpressure and upstream-failure propagation need the TS channel runtime
/// and are not modeled here.
pub fn parse_multipart(
    body: &[u8],
    content_type: &str,
    limits: &MultipartLimits,
) -> Result<Vec<MultipartPart>, MultipartError> {
    let boundary = multipart_boundary(content_type)
        .ok_or(MultipartError::from_reason(MultipartReason::Parse))?;
    if let Some(cap) = limits.max_total_size {
        if body.len() > cap {
            return Err(MultipartError::from_reason(MultipartReason::BodyTooLarge));
        }
    }
    let delimiter = format!("--{boundary}");
    let delim = delimiter.as_bytes();
    if !body.starts_with(delim) {
        return Err(MultipartError::from_reason(MultipartReason::Parse));
    }
    let field_mimes = limits.field_mimes();
    let mut parts = Vec::new();
    let mut pos = delim.len();
    // First delimiter line: either `--` (empty close) or `\r\n`.
    if body.get(pos..pos + 2) == Some(b"--".as_slice()) {
        return Ok(parts);
    }
    if body.get(pos..pos + 2) != Some(b"\r\n".as_slice()) {
        return Err(MultipartError::from_reason(MultipartReason::Parse));
    }
    pos += 2;
    loop {
        let header_end = find_bytes(body, pos, b"\r\n\r\n")
            .ok_or(MultipartError::from_reason(MultipartReason::Parse))?;
        let headers = String::from_utf8_lossy(&body[pos..header_end]).into_owned();
        let content_start = header_end + 4;
        let delim_pos = find_delimiter(body, content_start, delim)
            .ok_or(MultipartError::from_reason(MultipartReason::Parse))?;
        let content = &body[content_start..delim_pos];
        let (name, filename, content_type) = part_headers(&headers);
        let name = name.ok_or(MultipartError::from_reason(MultipartReason::Parse))?;
        let is_field = filename.is_none() || field_mimes.iter().any(|m| content_type.contains(m));
        if is_field {
            if content.len() > limits.field_cap() {
                return Err(MultipartError::from_reason(MultipartReason::FieldTooLarge));
            }
            parts.push(MultipartPart::Field {
                key: name,
                content_type,
                value: String::from_utf8_lossy(content).into_owned(),
            });
        } else {
            if let Some(cap) = limits.max_file_size {
                if content.len() > cap {
                    return Err(MultipartError::from_reason(MultipartReason::FileTooLarge));
                }
            }
            parts.push(MultipartPart::File {
                key: name.clone(),
                name: filename.unwrap_or(name),
                content_type,
                content: content.to_vec(),
            });
        }
        if let Some(cap) = limits.max_parts {
            if parts.len() > cap {
                return Err(MultipartError::from_reason(MultipartReason::TooManyParts));
            }
        }
        let mut next = delim_pos + 2 + delim.len();
        if body.get(next..next + 2) == Some(b"--".as_slice()) {
            break;
        }
        if body.get(next..next + 2) != Some(b"\r\n".as_slice()) {
            return Err(MultipartError::from_reason(MultipartReason::Parse));
        }
        pos = next + 2;
    }
    Ok(parts)
}

fn find_bytes(haystack: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || from > haystack.len() {
        return None;
    }
    haystack[from..]
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|i| from + i)
}

fn find_delimiter(body: &[u8], from: usize, delim: &[u8]) -> Option<usize> {
    // Delimiters are always introduced by `\r\n` (the separator is framing,
    // never file content), which also makes a split trailing `--` rejoin
    // harmless once chunks are reassembled.
    let mut search = from;
    loop {
        let hit = find_bytes(body, search, delim)?;
        let is_framing = hit >= 2 && body[hit - 2..hit] == *b"\r\n";
        let at_start = hit == 0;
        if is_framing || at_start {
            if is_framing {
                return Some(hit - 2);
            }
            return Some(hit);
        }
        search = hit + 1;
    }
}

fn part_headers(headers: &str) -> (Option<String>, Option<String>, String) {
    let mut name: Option<String> = None;
    let mut filename: Option<String> = None;
    let mut content_type = "text/plain".to_string();
    for line in headers.split("\r\n") {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim().to_ascii_lowercase();
        let value = value.trim();
        if key == "content-disposition" {
            for piece in value.split(';') {
                let piece = piece.trim();
                if let Some(rest) = piece.strip_prefix("name=") {
                    name = Some(unquote(rest));
                } else if let Some(rest) = piece.strip_prefix("filename=") {
                    filename = Some(unquote(rest));
                } else if let Some(rest) = piece.strip_prefix("filename*=") {
                    filename = Some(decode_rfc5987(rest));
                }
            }
        } else if key == "content-type" {
            content_type = value.split(';').next().unwrap_or(value).trim().to_string();
        }
    }
    (name, filename, content_type)
}

fn unquote(text: &str) -> String {
    let text = text.trim();
    if text.len() >= 2 && text.starts_with('"') && text.ends_with('"') {
        text[1..text.len() - 1].to_string()
    } else {
        text.to_string()
    }
}

fn decode_rfc5987(text: &str) -> String {
    // `UTF-8''%E6%97%A5...`: charset, language, then percent-encoding.
    let encoded = text.split('\'').nth(2).unwrap_or(text);
    let mut out = Vec::new();
    let bytes = encoded.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                out.push(hi << 4 | lo);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Parse JSON text then apply `reviver(key, value)` bottom-up, like
/// `JSON.parse` (TS `schemaJson` reviver core; schema validation itself
/// stays with the schema crate).
pub fn schema_json_with_reviver(
    json: &str,
    reviver: impl Fn(&str, serde_json::Value) -> serde_json::Value,
) -> Result<serde_json::Value, String> {
    let value: serde_json::Value = serde_json::from_str(json).map_err(|error| error.to_string())?;
    Ok(apply_reviver("", value, &reviver))
}

fn apply_reviver(
    key: &str,
    value: serde_json::Value,
    reviver: &impl Fn(&str, serde_json::Value) -> serde_json::Value,
) -> serde_json::Value {
    let value = match value {
        serde_json::Value::Array(items) => serde_json::Value::Array(
            items
                .into_iter()
                .enumerate()
                .map(|(i, item)| apply_reviver(&i.to_string(), item, reviver))
                .collect(),
        ),
        serde_json::Value::Object(map) => serde_json::Value::Object(
            map.into_iter()
                .map(|(k, v)| {
                    let v = apply_reviver(&k.clone(), v, reviver);
                    (k, v)
                })
                .collect(),
        ),
        other => other,
    };
    reviver(key, value)
}

/// Mint the next persisted path under `dir` for `filename` (TS `toPersisted`
/// naming: basename truncated to 128 chars, collisions get `N-` prefixes).
/// Appends to `used` in write order so callers can replay the write sequence.
pub fn persisted_file_path(
    dir: &str,
    filename: &str,
    used: &mut std::collections::BTreeSet<String>,
    index: &mut usize,
) -> String {
    let base = filename.rsplit(['/', '\\']).next().unwrap_or(filename);
    let base: String = base
        .chars()
        .rev()
        .take(128)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    let mut path = format!("{dir}/{base}");
    while used.contains(&path) {
        path = format!("{dir}/{}-{base}", *index);
        *index += 1;
    }
    used.insert(path.clone());
    path
}

/// The JSON Schema document for `PersistedFile` (TS `FileSchema.toJsonSchema`).
pub fn persisted_file_json_schema() -> serde_json::Value {
    serde_json::json!({
        "dialect": "draft-2020-12",
        "schema": {
            "type": "object",
            "properties": {
                "key": { "type": "string" },
                "name": { "type": "string" },
                "contentType": { "type": "string", "contentEncoding": "binary" },
                "path": { "type": "string" }
            },
            "required": ["key", "name", "contentType", "path"],
            "additionalProperties": true
        },
        "definitions": {}
    })
}
