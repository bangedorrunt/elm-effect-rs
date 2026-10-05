//! Developer tools events: spans and messages over an in-memory channel.
//!
//! A [`DevToolsServer`] collects [`DevEvent`]s; [`DevToolsClient`] handles
//! send them. The pair stands in for the DevTools wire protocol: same
//! shapes (span start/end, log message), no socket.

use std::collections::VecDeque;
use std::sync::Mutex;

/// One traced happening: a span boundary or a log line.
#[derive(Debug, Clone, PartialEq)]
pub enum DevEvent {
    /// A span opened: `id` nests inside `parent`.
    SpanStart {
        /// The new span id.
        id: u64,
        /// The enclosing span, if any.
        parent: Option<u64>,
        /// What the span names.
        name: String,
    },
    /// A span closed.
    SpanEnd {
        /// The span id from its start.
        id: u64,
    },
    /// A log line attached to a span (or none).
    Message {
        /// The enclosing span, if any.
        span: Option<u64>,
        /// The text.
        text: String,
    },
}

/// The collecting end: buffers every event in order.
///
/// ```
/// use elm_effect_unstable::devtools::{DevToolsClient, DevToolsServer};
/// let server = DevToolsServer::new();
/// let client = server.client();
/// let span = client.span_start(None, "work");
/// client.message(Some(span), "halfway");
/// client.span_end(span);
/// let events = server.drain();
/// assert_eq!(events.len(), 3);
/// ```
pub struct DevToolsServer {
    buffer: std::sync::Arc<Mutex<VecDeque<DevEvent>>>,
}

/// The sending end: cheaply clonable, pushes into the server buffer.
pub struct DevToolsClient {
    buffer: std::sync::Arc<Mutex<VecDeque<DevEvent>>>,
    next_id: std::sync::Arc<Mutex<u64>>,
    enabled: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl DevToolsServer {
    /// An empty collector.
    pub fn new() -> Self {
        Self {
            buffer: std::sync::Arc::new(Mutex::new(VecDeque::new())),
        }
    }

    /// A client handle feeding this server (shares the buffer).
    pub fn client(&self) -> DevToolsClient {
        DevToolsClient {
            buffer: self.buffer.clone(),
            next_id: std::sync::Arc::new(Mutex::new(1)),
            enabled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true)),
        }
    }

    /// Take every buffered event in order.
    pub fn drain(&self) -> Vec<DevEvent> {
        self.buffer.lock().unwrap().drain(..).collect()
    }
}

impl Default for DevToolsServer {
    fn default() -> Self {
        Self::new()
    }
}

impl DevToolsClient {
    /// Open a span, returning its id. Disabled clients drop the event.
    pub fn span_start(&self, parent: Option<u64>, name: &str) -> u64 {
        let mut next = self.next_id.lock().unwrap();
        let id = *next;
        *next += 1;
        if !self.is_enabled() {
            return id;
        }
        self.buffer.lock().unwrap().push_back(DevEvent::SpanStart {
            id,
            parent,
            name: name.to_string(),
        });
        id
    }

    /// Close a span. Disabled clients drop the event.
    pub fn span_end(&self, id: u64) {
        if !self.is_enabled() {
            return;
        }
        self.buffer
            .lock()
            .unwrap()
            .push_back(DevEvent::SpanEnd { id });
    }

    /// Log a line. Disabled clients drop the event.
    pub fn message(&self, span: Option<u64>, text: &str) {
        if !self.is_enabled() {
            return;
        }
        self.buffer.lock().unwrap().push_back(DevEvent::Message {
            span,
            text: text.to_string(),
        });
    }

    /// Take every event this client recorded.
    pub fn drain(&self) -> Vec<DevEvent> {
        self.buffer.lock().unwrap().drain(..).collect()
    }

    /// Flip recording on or off (a client-side state).
    pub fn set_enabled(&self, enabled: bool) {
        self.enabled
            .store(enabled, std::sync::atomic::Ordering::SeqCst);
    }

    /// Whether this client currently records.
    pub fn is_enabled(&self) -> bool {
        self.enabled.load(std::sync::atomic::Ordering::SeqCst)
    }
}

/// Wire-protocol schema for [`DevEvent`]: one line per event (the
/// `DevToolsSchema` shape, without a JSON dependency). Payloads are
/// length-prefixed so names and texts may hold any characters.
///
/// ```
/// use elm_effect_unstable::devtools::{DevEvent, decode, encode};
/// let event = DevEvent::SpanStart { id: 1, parent: None, name: "a b".to_string() };
/// assert_eq!(decode(&encode(&event)), Some(event));
/// ```
pub fn encode(event: &DevEvent) -> String {
    match event {
        DevEvent::SpanStart { id, parent, name } => {
            let scope = parent.map_or("-".to_string(), |p| p.to_string());
            format!("S {id} {scope} {}:{name}", name.len())
        }
        DevEvent::SpanEnd { id } => format!("E {id}"),
        DevEvent::Message { span, text } => {
            let scope = span.map_or("-".to_string(), |s| s.to_string());
            format!("M {scope} {}:{text}", text.len())
        }
    }
}

/// Parse one [`encode`] line; anything malformed is `None`.
pub fn decode(line: &str) -> Option<DevEvent> {
    let (kind, rest) = line.split_once(' ')?;
    match kind {
        "E" => rest.parse::<u64>().ok().map(|id| DevEvent::SpanEnd { id }),
        "S" => {
            let (id, rest) = rest.split_once(' ')?;
            let (parent, rest) = rest.split_once(' ')?;
            let (len, body) = rest.split_once(':')?;
            let id = id.parse::<u64>().ok()?;
            let parent = if parent == "-" {
                None
            } else {
                Some(parent.parse::<u64>().ok()?)
            };
            Some(DevEvent::SpanStart {
                id,
                parent,
                name: split_len_body(len, body)?.to_string(),
            })
        }
        "M" => {
            let (scope, rest) = rest.split_once(' ')?;
            let (len, body) = rest.split_once(':')?;
            let scope = if scope == "-" {
                None
            } else {
                Some(scope.parse::<u64>().ok()?)
            };
            Some(DevEvent::Message {
                span: scope,
                text: split_len_body(len, body)?.to_string(),
            })
        }
        _ => None,
    }
}

/// Split a `len:body` tail, rejecting short, long, and cut-UTF-8 bodies.
fn split_len_body<'a>(len: &str, body: &'a str) -> Option<&'a str> {
    let text = body.get(..len.parse::<usize>().ok()?)?;
    if body.len() != text.len() {
        return None;
    }
    Some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_lived_span_sends_once_in_each_state() {
        let server = DevToolsServer::new();
        let client = server.client();
        let span = client.span_start(None, "work");
        client.span_end(span);
        assert_eq!(server.drain().len(), 2);
        client.set_enabled(false);
        assert!(!client.is_enabled());
        let span = client.span_start(None, "work");
        client.message(Some(span), "halfway");
        client.span_end(span);
        assert!(server.drain().is_empty());
        client.set_enabled(true);
        let span = client.span_start(None, "work");
        client.span_end(span);
        assert_eq!(server.drain().len(), 2);
    }

    #[test]
    fn wire_round_trip_preserves_every_event_shape() {
        let events = [
            DevEvent::SpanStart {
                id: 1,
                parent: None,
                name: "a:b".to_string(),
            },
            DevEvent::SpanStart {
                id: 2,
                parent: Some(1),
                name: "".to_string(),
            },
            DevEvent::SpanEnd { id: 1 },
            DevEvent::Message {
                span: Some(2),
                text: "hi\nthere".to_string(),
            },
            DevEvent::Message {
                span: None,
                text: "".to_string(),
            },
        ];
        for event in events {
            assert_eq!(decode(&encode(&event)), Some(event));
        }
    }

    #[test]
    fn wire_decode_rejects_garbage() {
        for bad in [
            "",
            "nope",
            "S 1",
            "S x - 0:",
            "E",
            "E x",
            "M 1 5:abc",
            "S 1 - 9:short",
            "Z 1",
        ] {
            assert_eq!(decode(bad), None, "input {bad:?}");
        }
    }
}
