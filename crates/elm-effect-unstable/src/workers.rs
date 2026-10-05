//! OS threads behind a tiny join-handle surface.
//!
//! Thin over [`std::thread`]: spawn named threads, join them back,
//! sleep the current one. No pool, no executor.

/// A handle to a spawned thread: join it exactly once.
pub struct Worker<T> {
    inner: Option<std::thread::JoinHandle<T>>,
}

impl<T> Worker<T> {
    /// The thread's return value, blocking until done.
    /// Panics when the thread panicked, like [`std::thread`] does.
    pub fn join(mut self) -> T {
        self.inner.take().unwrap().join().unwrap()
    }
}

/// Spawn a thread running `work`, named `name` when given.
pub fn spawn<T>(name: Option<&str>, work: impl FnOnce() -> T + Send + 'static) -> Worker<T>
where
    T: Send + 'static,
{
    let builder = match name {
        Some(name) => std::thread::Builder::new().name(name.to_string()),
        None => std::thread::Builder::new(),
    };
    Worker {
        inner: Some(builder.spawn(work).unwrap()),
    }
}

/// Sleep the current thread for `millis` milliseconds.
pub fn sleep_millis(millis: u64) {
    std::thread::sleep(std::time::Duration::from_millis(millis));
}

/// The current thread's id, for debugging and logs.
pub fn current_id() -> String {
    format!("{:?}", std::thread::current().id())
}

/// Worker failure modes.
///
/// Ports `packages/effect/src/workers/WorkerError.ts`: spawn, send, and
/// receive failures plus an unknown-error catch-all, each carrying a message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkerError {
    /// Spawning the worker failed.
    Spawn(String),
    /// Sending a message to the worker failed.
    Send(String),
    /// Receiving a message from the worker failed.
    Receive(String),
    /// Any other worker failure.
    Unknown(String),
}

impl WorkerError {
    /// The failure message.
    pub fn message(&self) -> &str {
        match self {
            Self::Spawn(m) | Self::Send(m) | Self::Receive(m) | Self::Unknown(m) => m,
        }
    }

    /// True for spawn failures.
    pub fn is_spawn(&self) -> bool {
        matches!(self, Self::Spawn(_))
    }

    /// True for send failures.
    pub fn is_send(&self) -> bool {
        matches!(self, Self::Send(_))
    }

    /// True for receive failures.
    pub fn is_receive(&self) -> bool {
        matches!(self, Self::Receive(_))
    }
}

impl core::fmt::Display for WorkerError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let kind = match self {
            Self::Spawn(_) => "spawn",
            Self::Send(_) => "send",
            Self::Receive(_) => "receive",
            Self::Unknown(_) => "unknown",
        };
        write!(f, "WorkerError({kind}): {}", self.message())
    }
}

/// A value that can cross the worker boundary.
///
/// Ports the portable core of `Transferable.ts` (browser port/message-port
/// types have no host equivalent): inline bytes travel with the message,
/// larger buffers move through a [`Collector`] by handle id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Transferable {
    /// Small value carried inline.
    Bytes(Vec<u8>),
    /// Handle into a [`Collector`] holding the real buffer.
    Port(u64),
}

/// Registry for buffers shared with workers by handle.
///
/// Ports `makeCollector`: `offer` stores a buffer and returns its id,
/// `take` removes and returns it exactly once.
#[derive(Debug, Default)]
pub struct Collector {
    next: u64,
    buffers: std::collections::HashMap<u64, Vec<u8>>,
}

impl Collector {
    /// Empty collector.
    pub fn new() -> Self {
        Self::default()
    }

    /// Store a buffer, returning its handle.
    pub fn offer(&mut self, bytes: Vec<u8>) -> u64 {
        self.next += 1;
        self.buffers.insert(self.next, bytes);
        self.next
    }

    /// Remove and return the buffer for `id`, or `None` when absent
    /// (already taken or never offered).
    pub fn take(&mut self, id: u64) -> Option<Vec<u8>> {
        self.buffers.remove(&id)
    }

    /// Number of buffers currently held.
    pub fn len(&self) -> usize {
        self.buffers.len()
    }

    /// True when no buffers are held.
    pub fn is_empty(&self) -> bool {
        self.buffers.is_empty()
    }
}

/// A message arriving at the worker runner.
///
/// Ports `PlatformMessage` from `WorkerRunner.ts`: payloads travel to the
/// handler, `Close` stops the runner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlatformMessage {
    /// Application payload.
    Payload(Vec<u8>),
    /// Stop the runner after draining earlier messages.
    Close,
}

/// Drain `messages` in order, collecting payloads until `Close`.
/// Messages after `Close` are ignored. Returns the collected payloads.
pub fn run_runner(messages: &[PlatformMessage]) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    for message in messages {
        match message {
            PlatformMessage::Payload(bytes) => out.push(bytes.clone()),
            PlatformMessage::Close => break,
        }
    }
    out
}
