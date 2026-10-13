//! Socket pair (ADR-0002 Phase 12 platform substitutes).
//!
//! [`socket_pair`] links two [`End`]s: what one writes, the other reads,
//! one buffered message at a time. Writers park while the peer buffer
//! holds at least `high_water_mark` bytes (text counts UTF-8 bytes).
//! Dropping an end closes it: the peer drains what remains, then reads
//! resolve [`None`]. [`End::reader_string`] maps pulls to UTF-8 text.
//!
//! One state lock guards queues, counts, and flags together with a
//! single [`Condvar`](std::sync::Condvar): check-and-wait is atomic, so
//! no wakeup is ever lost between drain and park.
//!
//! The second half is `Socket.ts` itself: [`Socket`] pairs an acquirable
//! [`Reader`] (pull plus the TLS upgrade hook) with a [`Writer`],
//! [`Socket::from_websocket`] adds the web-socket shim's backpressure, and
//! [`Socket::to_channel`] writes through the writer. A pull that would
//! suspend at the pin reports [`Pull::Empty`] instead.

use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex};

/// Which side of the pair.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Side {
    /// First end.
    A,
    /// Second end.
    B,
}

struct State {
    a_to_b: VecDeque<Vec<u8>>,
    b_to_a: VecDeque<Vec<u8>>,
    buffered_a_to_b: usize,
    buffered_b_to_a: usize,
    closed_a: bool,
    closed_b: bool,
}

struct PairChannel {
    state: Mutex<State>,
    high_water: usize,
    wake: Condvar,
}

impl PairChannel {
    fn peer_closed(&self, state: &State, side: Side) -> bool {
        match side {
            Side::A => state.closed_b,
            Side::B => state.closed_a,
        }
    }
}

/// One end of a socket pair. Clone shares the end.
#[derive(Clone)]
pub struct End {
    channel: Arc<PairChannel>,
    side: Side,
}

impl End {
    /// Write one message; parks while the peer buffer is at or above
    /// the high-water mark. A closed peer ends the write silently.
    pub fn write(&self, bytes: Vec<u8>) {
        let mut state = self.channel.state.lock().unwrap();
        loop {
            if PairChannel::peer_closed(&self.channel, &state, self.side) {
                return;
            }
            let buffered = match self.side {
                Side::A => state.buffered_a_to_b,
                Side::B => state.buffered_b_to_a,
            };
            if buffered < self.channel.high_water {
                break;
            }
            state = self.channel.wake.wait(state).unwrap();
        }
        // Re-check under the same lock: no lost wakeup possible.
        if PairChannel::peer_closed(&self.channel, &state, self.side) {
            return;
        }
        let len = bytes.len();
        match self.side {
            Side::A => {
                state.a_to_b.push_back(bytes);
                state.buffered_a_to_b += len;
            }
            Side::B => {
                state.b_to_a.push_back(bytes);
                state.buffered_b_to_a += len;
            }
        }
        self.channel.wake.notify_all();
    }

    /// Next buffered message, or [`None`] once the peer closed and the
    /// buffer drained.
    pub fn reader_bytes(&self) -> Option<Vec<u8>> {
        let mut state = self.channel.state.lock().unwrap();
        loop {
            let msg = match self.side {
                Side::A => state.b_to_a.pop_front(),
                Side::B => state.a_to_b.pop_front(),
            };
            if let Some(msg) = msg {
                match self.side {
                    Side::A => state.buffered_b_to_a -= msg.len(),
                    Side::B => state.buffered_a_to_b -= msg.len(),
                }
                self.channel.wake.notify_all();
                return Some(msg);
            }
            if PairChannel::peer_closed(&self.channel, &state, self.side) {
                return None;
            }
            state = self.channel.wake.wait(state).unwrap();
        }
    }

    /// Next buffered message, waiting at most `timeout`.
    pub fn reader_bytes_timeout(&self, timeout: std::time::Duration) -> Option<Vec<u8>> {
        let mut state = self.channel.state.lock().unwrap();
        loop {
            let msg = match self.side {
                Side::A => state.b_to_a.pop_front(),
                Side::B => state.a_to_b.pop_front(),
            };
            if let Some(msg) = msg {
                match self.side {
                    Side::A => state.buffered_b_to_a -= msg.len(),
                    Side::B => state.buffered_a_to_b -= msg.len(),
                }
                self.channel.wake.notify_all();
                return Some(msg);
            }
            if PairChannel::peer_closed(&self.channel, &state, self.side) {
                return None;
            }
            let (guard, result) = self.channel.wake.wait_timeout(state, timeout).unwrap();
            state = guard;
            if result.timed_out() {
                return None;
            }
        }
    }

    /// Next buffered message as UTF-8 text.
    pub fn reader_string(&self) -> Option<Result<String, String>> {
        self.reader_bytes()
            .map(|bytes| String::from_utf8(bytes).map_err(|_| "invalid UTF-8".to_string()))
    }
}

impl Drop for End {
    fn drop(&mut self) {
        let mut state = self.channel.state.lock().unwrap();
        match self.side {
            Side::A => state.closed_a = true,
            Side::B => state.closed_b = true,
        }
        self.channel.wake.notify_all();
    }
}

// -------------------------------------------------------------- Socket.ts ---

/// Default high-water mark for pausable web sockets (pin: 64 KiB).
pub const DEFAULT_HIGH_WATER_MARK: usize = 64 * 1024;

/// Web-socket event kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EventKind {
    /// The connection opened.
    Open,
    /// A frame arrived.
    Message,
    /// The connection errored.
    Error,
    /// The connection closed.
    Close,
}

/// Event payload delivered to listeners.
#[derive(Clone, Debug, PartialEq)]
pub enum WebSocketEvent {
    /// Opened.
    Open,
    /// One frame (the port keeps the wire bytes; the pin also takes text).
    Message(Vec<u8>),
    /// Errored, with the cause text.
    Error(String),
    /// Closed, with code and reason.
    Close {
        /// Close code.
        code: u16,
        /// Close reason.
        reason: String,
    },
}

/// One attached listener; identity is the `Arc` pointer.
pub type WebSocketListener = Arc<dyn Fn(&WebSocketEvent) + Send + Sync>;

/// The web-socket surface [`Socket::from_websocket`] drives (pin
/// `WebSocketLike`).
pub trait WebSocketLike: Send + Sync {
    /// `0` connecting, `1` open, `2` or more closing/closed.
    fn ready_state(&self) -> u8;
    /// `true` when the socket supports backpressure (`isPausable`).
    fn pausable(&self) -> bool;
    /// Attach `listener`; a `once` listener is dropped after one dispatch.
    fn add_event_listener(&self, kind: EventKind, once: bool, listener: WebSocketListener);
    /// Detach `listener` by identity.
    fn remove_event_listener(&self, kind: EventKind, listener: &WebSocketListener);
    /// Stop reads (the pin's `pause`).
    fn pause(&self);
    /// Resume reads (the pin's `resume`).
    fn resume(&self);
    /// Send one chunk; `Err` text becomes a write failure.
    fn send(&self, chunk: &[u8]) -> Result<(), String>;
}

/// One pull outcome.
#[derive(Clone, Debug, PartialEq)]
pub enum Pull {
    /// The buffered frames, oldest first (the pin's non-empty array).
    Chunks(Vec<Vec<u8>>),
    /// Nothing buffered and no failure: the pin suspends the pull here.
    Empty,
    /// The read failed and keeps failing.
    Failed(SocketError),
}

/// Socket failure (`SocketError` at the pin). Error events flatten the pin's
/// read/open split because no carried row distinguishes the two.
#[derive(Clone, Debug, PartialEq)]
pub enum SocketError {
    /// Closed (pin `SocketCloseError`).
    Close {
        /// Close code (`1006` when the peer gave none).
        code: u16,
        /// Close reason.
        reason: String,
    },
    /// An error event arrived (pin `SocketReadError`/`SocketOpenError`).
    Read(String),
    /// A write threw (pin `SocketWriteError`).
    Write(String),
    /// The reader was acquired while still connecting; the pin suspends the
    /// acquisition here instead.
    Connecting,
    /// TLS upgrade is unavailable (`SocketUpgradeError.unsupported`).
    UpgradeUnsupported,
}

/// The reader's pull closure.
type PullFn = Arc<dyn Fn() -> Pull + Send + Sync>;

/// The reader's TLS upgrade hook.
type UpgradeFn = Arc<dyn Fn() -> Result<(), SocketError> + Send + Sync>;

/// The writer's write closure.
type WriteFn = Arc<dyn Fn(&[u8]) -> Result<(), SocketError> + Send + Sync>;

/// Reader half: one pull plus the TLS upgrade hook.
#[derive(Clone)]
pub struct Reader {
    pull: PullFn,
    upgrade: UpgradeFn,
}

impl Reader {
    /// From a pull and an upgrade hook.
    pub fn new(
        pull: impl Fn() -> Pull + Send + Sync + 'static,
        upgrade: impl Fn() -> Result<(), SocketError> + Send + Sync + 'static,
    ) -> Self {
        Self {
            pull: Arc::new(pull),
            upgrade: Arc::new(upgrade),
        }
    }

    /// One pull (`Reader.pull`).
    pub fn pull(&self) -> Pull {
        (self.pull)()
    }

    /// Run the TLS upgrade (`Reader.upgrade`).
    pub fn upgrade(&self) -> Result<(), SocketError> {
        (self.upgrade)()
    }
}

impl std::fmt::Debug for Reader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Reader").finish_non_exhaustive()
    }
}

/// Writer half.
#[derive(Clone)]
pub struct Writer {
    write: WriteFn,
}

impl Writer {
    /// From a write function.
    pub fn new(write: impl Fn(&[u8]) -> Result<(), SocketError> + Send + Sync + 'static) -> Self {
        Self {
            write: Arc::new(write),
        }
    }

    /// Write one chunk (`Writer.write`).
    pub fn write(&self, chunk: &[u8]) -> Result<(), SocketError> {
        (self.write)(chunk)
    }
}

/// Where a socket's reader comes from.
#[derive(Clone)]
enum Source {
    Plain(Reader),
    WebSocket(Arc<WebSocketState>),
}

/// A socket: an acquirable reader plus a writer (pin `Socket.make`).
#[derive(Clone)]
pub struct Socket {
    source: Source,
    writer: Writer,
}

impl Socket {
    /// `Socket.make({ reader, writer })`.
    pub fn make(reader: Reader, writer: Writer) -> Self {
        Self {
            source: Source::Plain(reader),
            writer,
        }
    }

    /// `Socket.fromWebSocket(ws, { highWaterMark })`: attach the message,
    /// error, and close listeners, plus the one-shot open listener that a
    /// connecting acquisition owns.
    pub fn from_websocket(ws: Arc<dyn WebSocketLike>, high_water_mark: Option<usize>) -> Self {
        let state = WebSocketState::attach(ws, high_water_mark);
        Self {
            writer: state.writer(),
            source: Source::WebSocket(state),
        }
    }

    /// `socket.reader`: a plain socket hands its reader over; a web socket
    /// waits for open, fails after a close or error (removing the open
    /// listener), and reports [`SocketError::Connecting`] while it is still
    /// connecting.
    pub fn acquire_reader(&self) -> Result<Reader, SocketError> {
        match &self.source {
            Source::Plain(reader) => Ok(reader.clone()),
            Source::WebSocket(state) => state.reader(),
        }
    }

    /// `Socket.toChannel`.
    pub fn to_channel(&self) -> Channel {
        Channel {
            socket: self.clone(),
        }
    }
}

/// `Socket.toChannel`: writes ride the writer, reads acquire the reader.
#[derive(Clone)]
pub struct Channel {
    socket: Socket,
}

impl Channel {
    /// The pin's `Stream.runDrain` over `pipeThroughChannel`: write every
    /// chunk and report the first failure. The reader is never consulted, so
    /// a suspended pull cannot mask a write failure.
    pub fn write_all(&self, chunks: &[Vec<u8>]) -> Result<(), SocketError> {
        for chunk in chunks {
            self.socket.writer.write(chunk)?;
        }
        Ok(())
    }

}

#[derive(Default)]
struct WebSocketInner {
    open: bool,
    error: Option<SocketError>,
    buffer: Vec<Vec<u8>>,
    buffered: usize,
    paused: bool,
}

/// A web-socket-backed socket: frame buffer, backpressure, and the pending
/// open listener the pin's acquisition owns.
struct WebSocketState {
    ws: Arc<dyn WebSocketLike>,
    mark: Option<usize>,
    pausable: bool,
    inner: Mutex<WebSocketInner>,
    open_listener: Mutex<Option<WebSocketListener>>,
}

impl WebSocketState {
    fn attach(ws: Arc<dyn WebSocketLike>, high_water_mark: Option<usize>) -> Arc<Self> {
        let pausable = ws.pausable();
        let mark = high_water_mark.or(if pausable {
            Some(DEFAULT_HIGH_WATER_MARK)
        } else {
            None
        });
        let ready_state = ws.ready_state();
        let state = Arc::new(Self {
            ws: Arc::clone(&ws),
            mark,
            pausable,
            inner: Mutex::new(WebSocketInner {
                // `readyState === 1` means there is nothing to wait for.
                open: ready_state == 1,
                ..WebSocketInner::default()
            }),
            open_listener: Mutex::new(None),
        });
        if ready_state >= 2 {
            state.fail(close_error(1006, ""));
        }

        let message = Arc::clone(&state);
        ws.add_event_listener(
            EventKind::Message,
            false,
            Arc::new(move |event| {
                if let WebSocketEvent::Message(data) = event {
                    message.push(data.clone());
                }
            }),
        );
        let error = Arc::clone(&state);
        ws.add_event_listener(
            EventKind::Error,
            true,
            Arc::new(move |event| {
                let cause = match event {
                    WebSocketEvent::Error(cause) => cause.clone(),
                    other => format!("{other:?}"),
                };
                error.fail(SocketError::Read(cause));
            }),
        );
        let close = Arc::clone(&state);
        ws.add_event_listener(
            EventKind::Close,
            true,
            Arc::new(move |event| {
                let (code, reason) = match event {
                    WebSocketEvent::Close { code, reason } => (*code, reason.clone()),
                    _ => (1006, String::new()),
                };
                close.fail(close_error(code, &reason));
            }),
        );

        if ready_state == 0 {
            let opened = Arc::clone(&state);
            let listener: WebSocketListener = Arc::new(move |event| {
                if matches!(event, WebSocketEvent::Open) {
                    opened.inner.lock().unwrap().open = true;
                    // The shim drops a `once` listener; forget it here too so
                    // a later close removes nothing twice.
                    *opened.open_listener.lock().unwrap() = None;
                }
            });
            ws.add_event_listener(EventKind::Open, true, Arc::clone(&listener));
            *state.open_listener.lock().unwrap() = Some(listener);
        }
        state
    }

    fn writer(&self) -> Writer {
        let ws = Arc::clone(&self.ws);
        Writer::new(move |chunk| ws.send(chunk).map_err(SocketError::Write))
    }

    fn reader(self: &Arc<Self>) -> Result<Reader, SocketError> {
        let error = self.inner.lock().unwrap().error.clone();
        if let Some(error) = error {
            // The pin's `fail` cleans the pending open listener up first.
            self.remove_open_listener();
            return Err(error);
        }
        if !self.inner.lock().unwrap().open {
            return Err(SocketError::Connecting);
        }
        let state = Arc::clone(self);
        Ok(Reader::new(
            move || state.pull(),
            || Err(SocketError::UpgradeUnsupported),
        ))
    }

    fn push(&self, data: Vec<u8>) {
        let pause = {
            let mut inner = self.inner.lock().unwrap();
            inner.buffered += data.len();
            inner.buffer.push(data);
            let at_mark = self
                .mark
                .is_some_and(|mark| inner.buffered >= mark);
            if self.pausable && !inner.paused && at_mark {
                inner.paused = true;
                true
            } else {
                false
            }
        };
        if pause {
            self.ws.pause();
        }
    }

    fn take_buffer(&self) -> Vec<Vec<u8>> {
        let (batch, resumed) = {
            let mut inner = self.inner.lock().unwrap();
            let batch = std::mem::take(&mut inner.buffer);
            inner.buffered = 0;
            let resumed = inner.error.is_none() && inner.paused;
            if resumed {
                inner.paused = false;
            }
            (batch, resumed)
        };
        if resumed {
            self.ws.resume();
        }
        batch
    }

    fn pull(&self) -> Pull {
        let (has_frames, error) = {
            let inner = self.inner.lock().unwrap();
            (!inner.buffer.is_empty(), inner.error.clone())
        };
        if has_frames {
            return Pull::Chunks(self.take_buffer());
        }
        match error {
            Some(error) => Pull::Failed(error),
            None => Pull::Empty,
        }
    }

    fn fail(&self, error: SocketError) {
        self.inner.lock().unwrap().error.get_or_insert(error);
        self.remove_open_listener();
    }

    fn remove_open_listener(&self) {
        let listener = self.open_listener.lock().unwrap().take();
        if let Some(listener) = listener {
            self.ws.remove_event_listener(EventKind::Open, &listener);
        }
    }
}

fn close_error(code: u16, reason: &str) -> SocketError {
    SocketError::Close {
        code,
        reason: reason.to_string(),
    }
}

/// Linked pair sharing one high-water mark.
pub fn socket_pair(high_water: usize) -> (End, End) {
    let channel = Arc::new(PairChannel {
        state: Mutex::new(State {
            a_to_b: VecDeque::new(),
            b_to_a: VecDeque::new(),
            buffered_a_to_b: 0,
            buffered_b_to_a: 0,
            closed_a: false,
            closed_b: false,
        }),
        high_water: high_water.max(1),
        wake: Condvar::new(),
    });
    (
        End {
            channel: channel.clone(),
            side: Side::A,
        },
        End {
            channel,
            side: Side::B,
        },
    )
}
