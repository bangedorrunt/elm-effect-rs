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

/// A worker failure reason.
///
/// Ports the four `Schema.Error` reason classes in
/// `packages/effect/src/workers/WorkerError.ts` (`WorkerSpawnError`,
/// `WorkerSendError`, `WorkerReceiveError`, `WorkerUnknownError`): each is a
/// `{ _tag, message, cause? }` record. `cause` is carried opaquely as a
/// JSON value, matching the pin's `Schema.Defect()` encoding.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "_tag")]
pub enum WorkerErrorReason {
    /// Spawning the worker failed.
    #[serde(rename = "WorkerSpawnError")]
    Spawn {
        /// Failure message.
        message: String,
        /// Optional defect that caused the failure.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cause: Option<serde_json::Value>,
    },
    /// Sending a message to the worker failed.
    #[serde(rename = "WorkerSendError")]
    Send {
        /// Failure message.
        message: String,
        /// Optional defect that caused the failure.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cause: Option<serde_json::Value>,
    },
    /// Receiving a message from the worker failed.
    #[serde(rename = "WorkerReceiveError")]
    Receive {
        /// Failure message.
        message: String,
        /// Optional defect that caused the failure.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cause: Option<serde_json::Value>,
    },
    /// Any other worker failure.
    #[serde(rename = "WorkerUnknownError")]
    Unknown {
        /// Failure message.
        message: String,
        /// Optional defect that caused the failure.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cause: Option<serde_json::Value>,
    },
}

impl WorkerErrorReason {
    /// Spawn failure reason.
    pub fn spawn(message: &str) -> Self {
        Self::Spawn { message: message.to_owned(), cause: None }
    }
    /// Send failure reason.
    pub fn send(message: &str) -> Self {
        Self::Send { message: message.to_owned(), cause: None }
    }
    /// Receive failure reason.
    pub fn receive(message: &str) -> Self {
        Self::Receive { message: message.to_owned(), cause: None }
    }
    /// Unknown failure reason.
    pub fn unknown(message: &str) -> Self {
        Self::Unknown { message: message.to_owned(), cause: None }
    }
    /// Attach a defect cause.
    pub fn with_cause(mut self, cause: serde_json::Value) -> Self {
        match &mut self {
            Self::Spawn { cause: c, .. }
            | Self::Send { cause: c, .. }
            | Self::Receive { cause: c, .. }
            | Self::Unknown { cause: c, .. } => *c = Some(cause),
        }
        self
    }

    /// The `_tag` string.
    pub fn tag(&self) -> &'static str {
        match self {
            Self::Spawn { .. } => "WorkerSpawnError",
            Self::Send { .. } => "WorkerSendError",
            Self::Receive { .. } => "WorkerReceiveError",
            Self::Unknown { .. } => "WorkerUnknownError",
        }
    }
    /// The failure message.
    pub fn message(&self) -> &str {
        match self {
            Self::Spawn { message, .. }
            | Self::Send { message, .. }
            | Self::Receive { message, .. }
            | Self::Unknown { message, .. } => message,
        }
    }
    /// The optional defect cause.
    pub fn cause(&self) -> Option<&serde_json::Value> {
        match self {
            Self::Spawn { cause, .. }
            | Self::Send { cause, .. }
            | Self::Receive { cause, .. }
            | Self::Unknown { cause, .. } => cause.as_ref(),
        }
    }
    /// Encode to the pin's JSON form.
    pub fn encode(&self) -> serde_json::Value {
        serde_json::to_value(self).expect("reason encodes")
    }
    /// Decode from the pin's JSON form.
    pub fn decode(value: &serde_json::Value) -> Result<Self, String> {
        serde_json::from_value(value.clone()).map_err(|e| e.to_string())
    }
}

/// Worker failure wrapper.
///
/// Ports `WorkerError` in `packages/effect/src/workers/WorkerError.ts`: it
/// wraps a [`WorkerErrorReason`] and delegates `message`/`cause` to it. Encoded
/// form is `{ _tag: "WorkerError", reason: <reason> }`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerError {
    /// The underlying reason.
    pub reason: WorkerErrorReason,
}

impl WorkerError {
    /// Wrap a reason.
    pub fn new(reason: WorkerErrorReason) -> Self {
        Self { reason }
    }
    /// The failure message.
    pub fn message(&self) -> &str {
        self.reason.message()
    }
    /// The optional defect cause.
    pub fn cause(&self) -> Option<&serde_json::Value> {
        self.reason.cause()
    }
    /// True for spawn failures.
    pub fn is_spawn(&self) -> bool {
        matches!(self.reason, WorkerErrorReason::Spawn { .. })
    }
    /// True for send failures.
    pub fn is_send(&self) -> bool {
        matches!(self.reason, WorkerErrorReason::Send { .. })
    }
    /// True for receive failures.
    pub fn is_receive(&self) -> bool {
        matches!(self.reason, WorkerErrorReason::Receive { .. })
    }
    /// The `_tag` string.
    pub fn tag(&self) -> &'static str {
        "WorkerError"
    }
    /// Encode to the pin's JSON form.
    pub fn encode(&self) -> serde_json::Value {
        serde_json::json!({ "_tag": "WorkerError", "reason": self.reason.encode() })
    }
    /// Decode from the pin's JSON form.
    pub fn decode(value: &serde_json::Value) -> Result<Self, String> {
        if value.get("_tag").and_then(|t| t.as_str()) != Some("WorkerError") {
            return Err("expected a WorkerError".to_owned());
        }
        let reason = value
            .get("reason")
            .ok_or_else(|| "WorkerError has no reason".to_owned())?;
        Ok(Self::new(WorkerErrorReason::decode(reason)?))
    }
}

impl core::fmt::Display for WorkerError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let kind = match self.reason {
            WorkerErrorReason::Spawn { .. } => "spawn",
            WorkerErrorReason::Send { .. } => "send",
            WorkerErrorReason::Receive { .. } => "receive",
            WorkerErrorReason::Unknown { .. } => "unknown",
        };
        write!(f, "WorkerError({kind}): {}", self.message())
    }
}

/// True only for a [`WorkerError`] itself.
///
/// Ports `isWorkerError`, which checks the `TypeId` brand: a bare reason, a
/// foreign error, or a look-alike `{ _tag: "WorkerError" }` all answer false.
pub fn is_worker_error<T: 'static>(_value: &T) -> bool {
    core::any::TypeId::of::<T>() == core::any::TypeId::of::<WorkerError>()
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
