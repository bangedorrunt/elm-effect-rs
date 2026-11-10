use crate::actor::{ActorDirective, ActorId, ActorLogic};
use crate::dynamic::DynamicEvent;

/// Action queue provided to transition, entry, and exit handlers.
///
/// Implements XState v6's unified enqueue discipline (`enq`), replacing
/// legacy action creators (`assign`, `raise`, `sendTo`).
#[derive(Debug, Default)]
pub struct ActionQueue<CMD, E> {
    pub(crate) commands: Vec<CMD>,
    pub(crate) raised: Vec<E>,
    pub(crate) emitted: Vec<serde_json::Value>,
    pub(crate) directives: Vec<ActorDirective>,
}

impl<CMD, E> ActionQueue<CMD, E> {
    /// Create a new empty action queue.
    #[must_use]
    pub fn new() -> Self {
        Self {
            commands: Vec::new(),
            raised: Vec::new(),
            emitted: Vec::new(),
            directives: Vec::new(),
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

    /// Drain all enqueued actor directives.
    pub fn take_directives(&mut self) -> Vec<ActorDirective> {
        std::mem::take(&mut self.directives)
    }
}

/// Actor verbs on a schema-driven action queue (ADR-0007 `enq.spawn`, `enq.send_to`, `enq.stop`).
///
/// These live on `ActionQueue<CMD, DynamicEvent>` because an [`ActorSystem`](crate::ActorSystem)
/// routes [`DynamicEvent`] values. A typed chart keeps its own event type and
/// does not use the actor verbs.
impl<CMD> ActionQueue<CMD, DynamicEvent> {
    /// Ask the host to create a child actor under the sending actor.
    pub fn spawn(&mut self, id: impl Into<ActorId>, logic: ActorLogic) {
        self.directives.push(ActorDirective::Spawn {
            id: id.into(),
            logic,
        });
    }

    /// Ask the host to deliver an event to another actor.
    pub fn send_to(&mut self, target: impl Into<ActorId>, event: DynamicEvent) {
        self.directives.push(ActorDirective::Send {
            target: target.into(),
            event,
        });
    }

    /// Ask the host to stop another actor and its descendants.
    pub fn stop(&mut self, id: impl Into<ActorId>) {
        self.directives.push(ActorDirective::Stop { id: id.into() });
    }
}
