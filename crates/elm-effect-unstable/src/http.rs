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
