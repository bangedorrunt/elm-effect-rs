/// Action queue provided to transition, entry, and exit handlers.
///
/// Implements XState v6's unified enqueue discipline (`enq`), replacing
/// legacy action creators (`assign`, `raise`, `sendTo`).
#[derive(Debug, Default)]
pub struct ActionQueue<CMD, E> {
    pub(crate) commands: Vec<CMD>,
    pub(crate) raised: Vec<E>,
    pub(crate) emitted: Vec<serde_json::Value>,
}

impl<CMD, E> ActionQueue<CMD, E> {
    /// Create a new empty action queue.
    #[must_use]
    pub fn new() -> Self {
        Self {
            commands: Vec::new(),
            raised: Vec::new(),
            emitted: Vec::new(),
        }
    }

    /// Enqueue a managed command for runtime execution.
    pub fn exec(&mut self, cmd: CMD) {
        self.commands.push(cmd);
    }

    /// Enqueue an internal event to immediately re-enter the machine fold (XState `enq.raise`).
    pub fn raise(&mut self, event: E) {
        self.raised.push(event);
    }

    /// Stream an inspection payload to external telemetry subscribers (XState `enq.emit`).
    pub fn emit(&mut self, payload: impl Into<serde_json::Value>) {
        self.emitted.push(payload.into());
    }

    /// Drain all enqueued runtime commands.
    pub fn take_commands(&mut self) -> Vec<CMD> {
        std::mem::take(&mut self.commands)
    }

    /// Drain all enqueued internal events.
    pub fn take_raised(&mut self) -> Vec<E> {
        std::mem::take(&mut self.raised)
    }

    /// Drain all enqueued inspection emissions.
    pub fn take_emitted(&mut self) -> Vec<serde_json::Value> {
        std::mem::take(&mut self.emitted)
    }
}
