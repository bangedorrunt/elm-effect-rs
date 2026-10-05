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
    /// Open a span, returning its id.
    pub fn span_start(&self, parent: Option<u64>, name: &str) -> u64 {
        let mut next = self.next_id.lock().unwrap();
        let id = *next;
        *next += 1;
        self.buffer.lock().unwrap().push_back(DevEvent::SpanStart {
            id,
            parent,
            name: name.to_string(),
        });
        id
    }

    /// Close a span.
    pub fn span_end(&self, id: u64) {
        self.buffer
            .lock()
            .unwrap()
            .push_back(DevEvent::SpanEnd { id });
    }

    /// Log a line.
    pub fn message(&self, span: Option<u64>, text: &str) {
        self.buffer.lock().unwrap().push_back(DevEvent::Message {
            span,
            text: text.to_string(),
        });
    }

    /// Take every event this client recorded.
    pub fn drain(&self) -> Vec<DevEvent> {
        self.buffer.lock().unwrap().drain(..).collect()
    }
}
