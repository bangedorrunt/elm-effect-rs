//! Rpc port: message envelopes, router, group, middleware, fake pair.
//!
//! [`Message`] is the wire envelope: requests carry an id plus a payload,
//! responses carry the id plus an [`Exit`] (success value, failure, or
//! defect). [`encode`] renders envelopes to JSON text; [`decode`] parses
//! them back with prototype-safe rules (inherited properties ignored,
//! non-array payloads wrapped, arrays kept flat). [`Router`] dispatches
//! by method name to handlers. [`Group`] collects route schemas.
//! [`Middleware`] wraps dispatch. [`FakePair`] is an in-memory
//! client/server transport: no sockets, no HTTP. JSON here is a minimal
//! hand codec (objects, arrays, strings with escapes, numbers, booleans,
//! null) so the crate stays dependency-free; full Schema payloads belong
//! to the schema wave.

use std::collections::BTreeMap;

/// JSON value (minimal codec model).
#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    /// Null.
    Null,
    /// Boolean.
    Bool(bool),
    /// Number (verbatim text).
    Num(String),
    /// String.
    Str(String),
    /// Array.
    Arr(Vec<Json>),
    /// Object (own properties only by construction).
    Obj(BTreeMap<String, Json>),
}

impl Json {
    /// Encode to JSON text.
    pub fn encode(&self) -> String {
        match self {
            Self::Null => "null".to_string(),
            Self::Bool(true) => "true".to_string(),
            Self::Bool(false) => "false".to_string(),
            Self::Num(n) => n.clone(),
            Self::Str(s) => encode_string(s),
            Self::Arr(items) => {
                format!(
                    "[{}]",
                    items
                        .iter()
                        .map(|v| v.encode())
                        .collect::<Vec<_>>()
                        .join(",")
                )
            }
            Self::Obj(map) => {
                let parts: Vec<String> = map
                    .iter()
                    .map(|(k, v)| format!("{}:{}", encode_string(k), v.encode()))
                    .collect();
                format!("{{{}}}", parts.join(","))
            }
        }
    }

    /// Decode JSON text; inherited properties cannot exist here because
    /// objects are built fresh from own parsed pairs.
    pub fn decode(text: &str) -> Result<Self, String> {
        let mut parser = Parser {
            bytes: text.as_bytes(),
            pos: 0,
        };
        parser.value().and_then(|v| {
            parser.ws();
            if parser.pos == parser.bytes.len() {
                Ok(v)
            } else {
                Err("trailing bytes".to_string())
            }
        })
    }
}

fn encode_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

struct Parser<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl Parser<'_> {
    fn ws(&mut self) {
        while self.pos < self.bytes.len() && (self.bytes[self.pos] as char).is_whitespace() {
            self.pos += 1;
        }
    }

    fn value(&mut self) -> Result<Json, String> {
        self.ws();
        match self.bytes.get(self.pos) {
            Some(b'n') => self.literal("null", Json::Null),
            Some(b't') => self.literal("true", Json::Bool(true)),
            Some(b'f') => self.literal("false", Json::Bool(false)),
            Some(b'"') => Ok(Json::Str(self.string()?)),
            Some(b'[') => self.array(),
            Some(b'{') => self.object(),
            Some(c) if (*c as char).is_ascii_digit() || *c == b'-' => self.number(),
            _ => Err("unexpected input".to_string()),
        }
    }

    fn literal(&mut self, word: &str, value: Json) -> Result<Json, String> {
        if self.bytes[self.pos..].starts_with(word.as_bytes()) {
            self.pos += word.len();
            Ok(value)
        } else {
            Err(format!("bad literal at {}", self.pos))
        }
    }

    fn string(&mut self) -> Result<String, String> {
        // Opening quote already peeked. Raw bytes accumulate so multibyte
        // sequences survive; UTF-8 decodes once at the close quote.
        self.pos += 1;
        let mut raw = Vec::new();
        loop {
            match self.bytes.get(self.pos) {
                None => return Err("unterminated string".to_string()),
                Some(b'"') => {
                    self.pos += 1;
                    return std::str::from_utf8(&raw)
                        .map(|text| text.to_string())
                        .map_err(|_| "invalid utf-8 in string".to_string());
                }
                Some(b'\\') => {
                    self.pos += 1;
                    match self.bytes.get(self.pos) {
                        Some(b'"') => raw.push(b'"'),
                        Some(b'\\') => raw.push(b'\\'),
                        Some(b'/') => raw.push(b'/'),
                        Some(b'n') => raw.push(b'\n'),
                        Some(b'r') => raw.push(b'\r'),
                        Some(b't') => raw.push(b'\t'),
                        Some(b'b') => raw.push(8),
                        Some(b'f') => raw.push(12),
                        Some(b'u') => {
                            if self.pos + 4 >= self.bytes.len() {
                                return Err("bad unicode escape".to_string());
                            }
                            let hex = std::str::from_utf8(&self.bytes[self.pos + 1..self.pos + 5])
                                .map_err(|_| "bad unicode escape".to_string())?;
                            let code = u32::from_str_radix(hex, 16)
                                .map_err(|_| "bad unicode escape".to_string())?;
                            let mut encoded = [0u8; 4];
                            raw.extend_from_slice(
                                char::from_u32(code)
                                    .unwrap_or('\u{FFFD}')
                                    .encode_utf8(&mut encoded)
                                    .as_bytes(),
                            );
                            self.pos += 4;
                        }
                        _ => return Err("bad escape".to_string()),
                    }
                    self.pos += 1;
                }
                Some(_) => {
                    raw.push(self.bytes[self.pos]);
                    self.pos += 1;
                }
            }
        }
    }

    fn array(&mut self) -> Result<Json, String> {
        self.pos += 1;
        let mut items = Vec::new();
        let mut first = true;
        loop {
            self.ws();
            if self.bytes.get(self.pos) == Some(&b']') {
                if first {
                    self.pos += 1;
                    return Ok(Json::Arr(items));
                }
                return Err("trailing comma".to_string());
            }
            first = false;
            items.push(self.value()?);
            self.ws();
            match self.bytes.get(self.pos) {
                Some(b',') => {
                    self.pos += 1;
                }
                Some(b']') => {
                    self.pos += 1;
                    return Ok(Json::Arr(items));
                }
                _ => return Err("bad array".to_string()),
            }
        }
    }

    fn object(&mut self) -> Result<Json, String> {
        self.pos += 1;
        let mut map = BTreeMap::new();
        let mut first = true;
        loop {
            self.ws();
            if self.bytes.get(self.pos) == Some(&b'}') {
                if first {
                    self.pos += 1;
                    return Ok(Json::Obj(map));
                }
                return Err("trailing comma".to_string());
            }
            first = false;
            if self.bytes.get(self.pos) != Some(&b'"') {
                return Err("bad object key".to_string());
            }
            let key = self.string()?;
            self.ws();
            if self.bytes.get(self.pos) != Some(&b':') {
                return Err("bad object".to_string());
            }
            self.pos += 1;
            let value = self.value()?;
            map.insert(key, value);
            self.ws();
            match self.bytes.get(self.pos) {
                Some(b',') => {
                    self.pos += 1;
                }
                Some(b'}') => {
                    self.pos += 1;
                    return Ok(Json::Obj(map));
                }
                _ => return Err("bad object".to_string()),
            }
        }
    }

    fn number(&mut self) -> Result<Json, String> {
        let start = self.pos;
        while self.pos < self.bytes.len()
            && "+-.0123456789eE".contains(self.bytes[self.pos] as char)
        {
            self.pos += 1;
        }
        Ok(Json::Num(
            String::from_utf8_lossy(&self.bytes[start..self.pos]).into_owned(),
        ))
    }
}

/// RPC exit shape.
#[derive(Clone, Debug, PartialEq)]
pub enum Exit {
    /// Success with value.
    Success(Json),
    /// Typed failure.
    Failure(Json),
    /// Defect (plain message).
    Defect(String),
    /// Interrupted.
    Interrupt,
}

/// Wire message envelope.
#[derive(Clone, Debug, PartialEq)]
pub enum Message {
    /// Request: id plus payload.
    Request {
        /// Request id (preserved verbatim, string or number).
        id: Json,
        /// Method name.
        method: String,
        /// Payload.
        payload: Json,
    },
    /// Response: id plus exit.
    Response {
        /// Request id echoed back.
        id: Json,
        /// Outcome.
        exit: Exit,
    },
}

impl Message {
    /// Encode to a JSON string.
    pub fn encode(&self) -> String {
        match self {
            Self::Request {
                id,
                method,
                payload,
            } => {
                let mut map = BTreeMap::new();
                map.insert("_tag".to_string(), Json::Str("Request".to_string()));
                map.insert("requestId".to_string(), id.clone());
                map.insert("method".to_string(), Json::Str(method.clone()));
                map.insert("payload".to_string(), payload.clone());
                Json::Obj(map).encode()
            }
            Self::Response { id, exit } => {
                let mut map = BTreeMap::new();
                map.insert("_tag".to_string(), Json::Str("Exit".to_string()));
                map.insert("requestId".to_string(), id.clone());
                map.insert("exit".to_string(), exit_json(exit));
                Json::Obj(map).encode()
            }
        }
    }

    /// Decode; unknown/inherited fields are ignored, never trusted.
    pub fn decode(text: &str) -> Result<Self, String> {
        let value = Json::decode(text)?;
        let obj = match value {
            Json::Obj(map) => map,
            _ => return Err("message must be an object".to_string()),
        };
        let tag = match obj.get("_tag") {
            Some(Json::Str(t)) => t.clone(),
            _ => return Err("message needs a _tag".to_string()),
        };
        let id = obj.get("requestId").cloned().unwrap_or(Json::Null);
        match tag.as_str() {
            "Request" => {
                let method = match obj.get("method") {
                    Some(Json::Str(m)) => m.clone(),
                    _ => return Err("request needs a method".to_string()),
                };
                // Non-array payloads wrap; arrays stay flat.
                let payload = match obj.get("payload") {
                    Some(Json::Arr(_)) => obj.get("payload").cloned().unwrap(),
                    Some(single) => Json::Arr(vec![single.clone()]),
                    None => Json::Arr(vec![]),
                };
                Ok(Self::Request {
                    id,
                    method,
                    payload,
                })
            }
            "Exit" => {
                let exit = match obj.get("exit") {
                    Some(Json::Obj(e)) => decode_exit(e)?,
                    _ => return Err("exit needs an object".to_string()),
                };
                Ok(Self::Response { id, exit })
            }
            other => Err(format!("unknown message tag: {other}")),
        }
    }
}

fn exit_json(exit: &Exit) -> Json {
    let mut map = BTreeMap::new();
    match exit {
        Exit::Success(value) => {
            map.insert("_tag".to_string(), Json::Str("Success".to_string()));
            map.insert("value".to_string(), value.clone());
        }
        Exit::Failure(value) => {
            map.insert("_tag".to_string(), Json::Str("Failure".to_string()));
            map.insert("value".to_string(), value.clone());
        }
        Exit::Defect(message) => {
            map.insert("_tag".to_string(), Json::Str("Defect".to_string()));
            map.insert("message".to_string(), Json::Str(message.clone()));
        }
        Exit::Interrupt => {
            map.insert("_tag".to_string(), Json::Str("Interrupt".to_string()));
        }
    }
    Json::Obj(map)
}

fn decode_exit(map: &BTreeMap<String, Json>) -> Result<Exit, String> {
    let missing = Json::Null;
    let tag = match map.get("_tag") {
        Some(Json::Str(t)) => t.as_str(),
        _ => return Err("exit needs a _tag".to_string()),
    };
    match tag {
        "Success" => Ok(Exit::Success(map.get("value").unwrap_or(&missing).clone())),
        "Failure" => Ok(Exit::Failure(map.get("value").unwrap_or(&missing).clone())),
        "Defect" => {
            // Interrupt exits use null for a missing message.
            let message = match map.get("message") {
                Some(Json::Str(m)) => m.clone(),
                _ => String::new(),
            };
            Ok(Exit::Defect(message))
        }
        "Interrupt" => Ok(Exit::Interrupt),
        other => Err(format!("unknown exit tag: {other}")),
    }
}

/// Route handler: payload in, exit out.
pub type Handler = Box<dyn Fn(Json) -> Exit + Send + Sync>;

/// Method router.
#[derive(Default)]
pub struct Router {
    routes: BTreeMap<String, Handler>,
}

impl Router {
    /// Empty router.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register one method handler.
    pub fn route(&mut self, method: &str, handler: impl Fn(Json) -> Exit + Send + Sync + 'static) {
        self.routes.insert(method.to_string(), Box::new(handler));
    }

    /// Dispatch a request message.
    pub fn dispatch(&self, msg: &Message) -> Message {
        match msg {
            Message::Request {
                id,
                method,
                payload,
            } => {
                let exit = match self.routes.get(method) {
                    Some(handler) => handler(payload.clone()),
                    None => Exit::Defect(format!("unknown method: {method}")),
                };
                Message::Response {
                    id: id.clone(),
                    exit,
                }
            }
            Message::Response { .. } => Message::Response {
                id: Json::Null,
                exit: Exit::Defect("router takes requests".to_string()),
            },
        }
    }
}

/// Named route schema (method plus description; payload shapes live with
/// the schema wave).
#[derive(Clone, Debug, PartialEq)]
pub struct RouteSchema {
    /// Method name.
    pub method: String,
    /// Human description.
    pub description: String,
}

/// Route group.
#[derive(Clone, Debug, Default)]
pub struct Group {
    /// Routes in registration order.
    pub routes: Vec<RouteSchema>,
}

impl Group {
    /// Empty group.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a route schema.
    pub fn route(&mut self, method: &str, description: &str) {
        self.routes.push(RouteSchema {
            method: method.to_string(),
            description: description.to_string(),
        });
    }

    /// Stream schemas carried by this group (none here: streams need the
    /// stream wave; recorded as an explicit empty set, not a silent skip).
    pub fn stream_schemas(&self) -> Vec<RouteSchema> {
        Vec::new()
    }
}

/// Middleware: wraps dispatch.
pub struct Middleware {
    run: Box<dyn Fn(&Message, &dyn Fn(&Message) -> Message) -> Message + Send + Sync>,
}

impl Middleware {
    /// Build from a wrapper.
    pub fn new(
        run: impl Fn(&Message, &dyn Fn(&Message) -> Message) -> Message + Send + Sync + 'static,
    ) -> Self {
        Self { run: Box::new(run) }
    }

    /// Run around `next`.
    pub fn run(&self, msg: &Message, next: &dyn Fn(&Message) -> Message) -> Message {
        (self.run)(msg, next)
    }
}

/// In-memory client/server pair: no sockets, no HTTP.
pub struct FakePair {
    router: Router,
    middlewares: Vec<Middleware>,
}

impl FakePair {
    /// Empty pair.
    pub fn new() -> Self {
        Self {
            router: Router::new(),
            middlewares: Vec::new(),
        }
    }

    /// Register a handler (server side).
    pub fn route(&mut self, method: &str, handler: impl Fn(Json) -> Exit + Send + Sync + 'static) {
        self.router.route(method, handler);
    }

    /// Add middleware (client side, applied in order).
    pub fn middleware(&mut self, middleware: Middleware) {
        self.middlewares.push(middleware);
    }

    /// Call: encode, run middlewares, dispatch, decode.
    pub fn call(&self, method: &str, payload: Json) -> Result<Message, String> {
        let request = Message::Request {
            id: Json::Num("1".to_string()),
            method: method.to_string(),
            payload,
        };
        let text = request.encode();
        let decoded = Message::decode(&text)?;
        Ok(self.apply(0, &decoded))
    }

    fn apply(&self, index: usize, msg: &Message) -> Message {
        match self.middlewares.get(index) {
            Some(middleware) => middleware.run(msg, &|m| self.apply(index + 1, m)),
            None => self.router.dispatch(msg),
        }
    }
}

/// One message per line over byte chunks (the ndjson framing shape).
///
/// Complete `\n`-terminated lines decode immediately; the tail waits for
/// more bytes, so a multibyte character split across chunks reassembles.
/// Lines that are not messages are skipped without losing the rest. When
/// `max_buffer` is set, an unterminated tail past that many bytes fails
/// the feed; `None` holds partial frames without bound.
#[derive(Debug, Default)]
pub struct NdjsonDecoder {
    tail: Vec<u8>,
    max_buffer: Option<usize>,
}

impl NdjsonDecoder {
    /// A decoder failing tails past `max_buffer` bytes (`None`: unbounded).
    pub fn new(max_buffer: Option<usize>) -> Self {
        Self {
            tail: Vec::new(),
            max_buffer,
        }
    }

    /// Feed bytes; returns the messages completed by this chunk.
    pub fn feed(&mut self, bytes: &[u8]) -> Result<Vec<Message>, String> {
        self.tail.extend_from_slice(bytes);
        let mut out = Vec::new();
        while let Some(end) = self.tail.iter().position(|&b| b == b'\n') {
            let mut line = self.tail.drain(..=end).collect::<Vec<u8>>();
            line.pop();
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            let text = match std::str::from_utf8(&line) {
                Ok(text) => text,
                Err(_) => continue,
            };
            if text.trim().is_empty() {
                continue;
            }
            if let Ok(message) = Message::decode(text) {
                out.push(message);
            }
        }
        match self.max_buffer {
            Some(max) if self.tail.len() > max => {
                return Err(format!("unterminated frame exceeds maxBufferSize {max}"));
            }
            _ => {}
        }
        Ok(out)
    }
}

/// Encode messages for the JSON-RPC shape: one message renders as its
/// object, several render as an array.
pub fn encode_batch(messages: &[Message]) -> String {
    if messages.len() == 1 {
        messages[0].encode()
    } else {
        format!(
            "[{}]",
            messages
                .iter()
                .map(|message| message.encode())
                .collect::<Vec<_>>()
                .join(",")
        )
    }
}

/// Decode a batch: objects decode alone, arrays decode per element with
/// non-objects skipped, anything else yields nothing.
pub fn decode_batch(text: &str) -> Vec<Message> {
    match Json::decode(text) {
        Ok(Json::Arr(items)) => items
            .iter()
            .filter_map(|item| Message::decode(&item.encode()).ok())
            .collect(),
        Ok(_) => Message::decode(text).ok().into_iter().collect(),
        Err(_) => Vec::new(),
    }
}
