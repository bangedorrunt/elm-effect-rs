use crate::configuration::Configuration;
use crate::dynamic::{DynamicEvent, DynamicMachine};
use crate::types::MachineState;
use elm_effect::{Defect, Effect, Scope};
use std::collections::{BTreeMap, VecDeque};
use std::fmt;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};

/// A stable actor name inside one [`ActorSystem`].
#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
pub struct ActorId(String);

impl ActorId {
    /// Name an actor.
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    /// The name as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ActorId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl From<&str> for ActorId {
    fn from(name: &str) -> Self {
        Self(name.to_string())
    }
}

impl From<String> for ActorId {
    fn from(name: String) -> Self {
        Self(name)
    }
}

/// A pure reducer actor: `(state, event) -> state`.
pub type Reducer =
    Arc<dyn Fn(&serde_json::Value, &DynamicEvent) -> serde_json::Value + Send + Sync>;

type EffectStart = Arc<dyn Fn() -> Effect<DynamicEvent, String> + Send + Sync>;

/// A one-shot host task. The actor system describes it; the host runs it.
#[derive(Clone)]
pub struct EffectLogic {
    start: EffectStart,
}

impl fmt::Debug for EffectLogic {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("EffectLogic")
    }
}

/// A non-blocking stream source. Each received event goes to the parent.
#[derive(Clone)]
pub struct StreamLogic {
    receiver: Arc<Mutex<Receiver<DynamicEvent>>>,
}

impl fmt::Debug for StreamLogic {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("StreamLogic")
    }
}

/// A callback invoked once at spawn with a sender to the actor's parent.
#[derive(Clone)]
pub struct CallbackLogic {
    start: Arc<dyn Fn(CallbackSender) + Send + Sync>,
}

impl fmt::Debug for CallbackLogic {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("CallbackLogic")
    }
}

/// The handle a callback actor receives. `send` forwards an event to the
/// parent; a stopped parent drops it.
#[derive(Clone, Debug)]
pub struct CallbackSender {
    actor: ActorId,
    tx: Sender<(ActorId, DynamicEvent)>,
}

impl CallbackSender {
    /// Forward one event to this actor's parent.
    ///
    /// Returns `false` when the system is gone.
    #[must_use]
    pub fn send(&self, event: DynamicEvent) -> bool {
        self.tx.send((self.actor.clone(), event)).is_ok()
    }

    /// The callback actor's name.
    #[must_use]
    pub fn actor(&self) -> &ActorId {
        &self.actor
    }
}

/// An effect the host must run for an actor, then report with
/// [`ActorSystem::complete`] or [`ActorSystem::fail`].
///
/// The host owns the runtime and runs the effect inside
/// [`ActorSystem::scope`] for the actor, so stopping the actor interrupts the
/// fiber. This crate only describes the task; it never runs one.
pub struct ActorTask {
    /// The actor the task belongs to.
    pub actor: ActorId,
    /// The effect to run.
    pub effect: Effect<DynamicEvent, String>,
}

/// What an actor computes.
///
/// Machine and reducer actors step inside the pure system. Effect, stream, and
/// callback actors are host-driven: the system describes their work and keeps
/// their lifetime, the host owns the runtime.
#[derive(Clone)]
pub enum ActorLogic {
    /// A schema-driven statechart.
    Machine(Arc<DynamicMachine>),
    /// A reducer with an initial value.
    Transition {
        /// The value before the first event.
        initial: serde_json::Value,
        /// The pure step function.
        reduce: Reducer,
    },
    /// A one-shot effect, from `from_effect` or `from_promise`.
    Effect(EffectLogic),
    /// A stream of events forwarded to the parent.
    Stream(StreamLogic),
    /// A callback that receives a sender at spawn.
    Callback(CallbackLogic),
}

impl ActorLogic {
    /// An actor whose behaviour is a [`DynamicMachine`].
    pub fn from_machine(machine: DynamicMachine) -> Self {
        Self::Machine(Arc::new(machine))
    }

    /// An actor whose behaviour is a pure reducer.
    pub fn from_transition(
        initial: serde_json::Value,
        reduce: impl Fn(&serde_json::Value, &DynamicEvent) -> serde_json::Value + Send + Sync + 'static,
    ) -> Self {
        Self::Transition {
            initial,
            reduce: Arc::new(reduce),
        }
    }

    /// A one-shot effect actor. Its task appears in
    /// [`ActorSystem::take_tasks`]; the host runs it and reports the outcome.
    pub fn from_effect(
        start: impl Fn() -> Effect<DynamicEvent, String> + Send + Sync + 'static,
    ) -> Self {
        Self::Effect(EffectLogic {
            start: Arc::new(start),
        })
    }

    /// A one-shot promise actor.
    ///
    /// This carrier has no separate `Future` type: an `Effect` is the async
    /// description a promise would resolve to. The completion contract is the
    /// same as [`from_effect`](Self::from_effect) — `xstate.done.actor.<id>`
    /// or `xstate.error.actor.<id>` once.
    pub fn from_promise(
        start: impl Fn() -> Effect<DynamicEvent, String> + Send + Sync + 'static,
    ) -> Self {
        Self::from_effect(start)
    }

    /// A non-blocking stream actor. Every received value is forwarded to the
    /// actor's parent; disconnect stops the actor.
    pub fn from_stream(receiver: Receiver<DynamicEvent>) -> Self {
        Self::Stream(StreamLogic {
            receiver: Arc::new(Mutex::new(receiver)),
        })
    }

    /// A callback actor. The handler runs once at spawn with a
    /// [`CallbackSender`] that forwards events to the parent.
    pub fn from_callback(handler: impl Fn(CallbackSender) + Send + Sync + 'static) -> Self {
        Self::Callback(CallbackLogic {
            start: Arc::new(handler),
        })
    }
}

impl fmt::Debug for ActorLogic {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Machine(_) => formatter.write_str("Machine"),
            Self::Transition { .. } => formatter.write_str("Transition"),
            Self::Effect(_) => formatter.write_str("Effect"),
            Self::Stream(_) => formatter.write_str("Stream"),
            Self::Callback(_) => formatter.write_str("Callback"),
        }
    }
}

/// Work an action asked the host actor system to do (XState `enq.spawn`,
/// `enq.send_to`, `enq.stop`).
///
/// A directive is data. The pure engine collects it; [`ActorSystem`] applies
/// it. An action never reaches another actor directly.
#[derive(Debug, Clone)]
pub enum ActorDirective {
    /// Create a child actor under the sender.
    Spawn {
        /// Name of the new actor.
        id: ActorId,
        /// What it computes.
        logic: ActorLogic,
    },
    /// Deliver an event to another actor's inbox.
    Send {
        /// Destination actor.
        target: ActorId,
        /// Event to deliver.
        event: DynamicEvent,
    },
    /// Stop one actor and its descendants.
    Stop {
        /// Actor to stop.
        id: ActorId,
    },
}

/// One actor's current value.
#[derive(Debug, Clone, PartialEq)]
pub enum ActorState {
    /// A statechart run.
    Machine(MachineState<String, crate::DynamicContext>),
    /// A reducer value.
    Transition(serde_json::Value),
    /// An effect, stream, or callback actor. The host holds its value.
    External,
}

impl ActorState {
    /// The statechart configuration, for a machine actor.
    #[must_use]
    pub fn configuration(&self) -> Option<&Configuration<String>> {
        match self {
            Self::Machine(state) => Some(&state.configuration),
            Self::Transition(_) | Self::External => None,
        }
    }

    /// The machine context, for a machine actor.
    #[must_use]
    pub fn context(&self) -> Option<&crate::DynamicContext> {
        match self {
            Self::Machine(state) => Some(&state.context),
            Self::Transition(_) | Self::External => None,
        }
    }

    /// The reducer value, for a transition actor.
    #[must_use]
    pub fn value(&self) -> Option<&serde_json::Value> {
        match self {
            Self::Machine(_) | Self::External => None,
            Self::Transition(value) => Some(value),
        }
    }
}

struct Actor {
    id: ActorId,
    parent: Option<ActorId>,
    logic: ActorLogic,
    state: ActorState,
    scope: Arc<Scope>,
}

struct Envelope {
    target: ActorId,
    event: DynamicEvent,
}

/// Why an actor operation failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ActorError {
    /// The name is already taken in this system.
    #[error("actor {0} already exists")]
    Duplicate(ActorId),
    /// No actor has this name.
    #[error("actor {0} does not exist")]
    Unknown(ActorId),
    /// The system did not reach quiescence inside its step budget.
    #[error("actor system did not quiesce within {0} steps")]
    StepBudget(usize),
}

/// In-process actor system with parent-child supervision.
///
/// Every actor owns a child [`Scope`] of its parent's scope. Stopping an actor
/// closes its scope, which interrupts its members and runs its finalizers
/// last-in-first-out, then repeats for every descendant. The system is pure:
/// it runs no fibers and reads no clock. A machine actor's `after` timers and
/// commands stay in its [`StepOutcome`](crate::StepOutcome) for the host.
///
/// ```no_run
/// use elm_effect_xstate::prelude::*;
///
/// let machine = DynamicMachine::from_json_str(r#"{"initial":"Idle","states":{"Idle":{}}}"#)?;
/// let mut system = ActorSystem::new("root", ActorLogic::from_machine(machine));
/// system.run()?;
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub struct ActorSystem {
    actors: BTreeMap<ActorId, Actor>,
    inbox: VecDeque<Envelope>,
    pending: VecDeque<(ActorId, ActorDirective)>,
    tasks: Vec<ActorTask>,
    callback_tx: Sender<(ActorId, DynamicEvent)>,
    callback_rx: Receiver<(ActorId, DynamicEvent)>,
    defects: Vec<Defect>,
    max_steps: usize,
}

const DEFAULT_STEP_BUDGET: usize = 10_000;

impl ActorSystem {
    /// A system with one root actor.
    pub fn new(root: impl Into<ActorId>, logic: ActorLogic) -> Self {
        let (callback_tx, callback_rx) = mpsc::channel();
        let mut system = Self {
            actors: BTreeMap::new(),
            inbox: VecDeque::new(),
            pending: VecDeque::new(),
            tasks: Vec::new(),
            callback_tx,
            callback_rx,
            defects: Vec::new(),
            max_steps: DEFAULT_STEP_BUDGET,
        };
        system.add_actor(root.into(), None, logic, Arc::new(Scope::new(0)));
        system
    }

    /// Resume a machine actor from a serialized run state instead of entering.
    ///
    /// The snapshot restores the configuration, recorded history, and context;
    /// the active states' entry actions do not run again. Children, streams,
    /// callbacks, and `after` timers are not in the snapshot: the host
    /// respawns them.
    pub fn resume(
        root: impl Into<ActorId>,
        machine: DynamicMachine,
        state: MachineState<String, crate::DynamicContext>,
    ) -> Self {
        let (callback_tx, callback_rx) = mpsc::channel();
        let id = root.into();
        let mut actors = BTreeMap::new();
        actors.insert(
            id.clone(),
            Actor {
                id,
                parent: None,
                logic: ActorLogic::Machine(Arc::new(machine)),
                state: ActorState::Machine(state),
                scope: Arc::new(Scope::new(0)),
            },
        );
        Self {
            actors,
            inbox: VecDeque::new(),
            pending: VecDeque::new(),
            tasks: Vec::new(),
            callback_tx,
            callback_rx,
            defects: Vec::new(),
            max_steps: DEFAULT_STEP_BUDGET,
        }
    }

    /// Bound the number of steps [`run`](Self::run) may take.
    #[must_use]
    pub fn with_max_steps(mut self, steps: usize) -> Self {
        self.max_steps = steps.max(1);
        self
    }

    /// Add a child actor under `parent`.
    ///
    /// # Errors
    ///
    /// Returns [`ActorError::Duplicate`] when the name is taken, or
    /// [`ActorError::Unknown`] when the parent does not exist.
    pub fn spawn(
        &mut self,
        parent: &ActorId,
        id: impl Into<ActorId>,
        logic: ActorLogic,
    ) -> Result<(), ActorError> {
        let id = id.into();
        if self.actors.contains_key(&id) {
            return Err(ActorError::Duplicate(id));
        }
        let Some(parent_actor) = self.actors.get(parent) else {
            return Err(ActorError::Unknown(parent.clone()));
        };
        let scope = parent_actor.scope.fork();
        self.add_actor(id, Some(parent.clone()), logic, scope);
        Ok(())
    }

    /// Queue one event for an actor.
    ///
    /// # Errors
    ///
    /// Returns [`ActorError::Unknown`] when the actor does not exist.
    pub fn send(
        &mut self,
        target: impl Into<ActorId>,
        event: DynamicEvent,
    ) -> Result<(), ActorError> {
        let target = target.into();
        if !self.actors.contains_key(&target) {
            return Err(ActorError::Unknown(target));
        }
        self.inbox.push_back(Envelope { target, event });
        Ok(())
    }

    /// Stop an actor and every descendant. Closing runs LIFO finalizers.
    ///
    /// Returns `false` when no actor has this name.
    pub fn stop(&mut self, id: &ActorId) -> bool {
        if !self.actors.contains_key(id) {
            return false;
        }
        let mut doomed = self.subtree(id);
        doomed.sort_by(|a, b| self.depth(b).cmp(&self.depth(a)).then_with(|| a.cmp(b)));
        for actor_id in doomed {
            if let Some(actor) = self.actors.remove(&actor_id) {
                self.defects.extend(actor.scope.close());
            }
        }
        true
    }

    /// Close the root scope and empty the system.
    pub fn shutdown(&mut self) {
        if let Some(root) = self.root() {
            self.stop(&root);
        }
    }

    /// Deliver queued events, poll streams and callbacks, and apply directives
    /// until nothing remains.
    ///
    /// # Errors
    ///
    /// Returns a routing error, or [`ActorError::StepBudget`] when the system
    /// does not settle inside its budget (a cycle is a bug, not a wait).
    pub fn run(&mut self) -> Result<(), ActorError> {
        for _ in 0..self.max_steps {
            if let Some((sender, directive)) = self.pending.pop_front() {
                self.apply(&sender, directive)?;
                continue;
            }
            if self.pump_streams() {
                continue;
            }
            if self.pump_callbacks() {
                continue;
            }
            let Some(envelope) = self.inbox.pop_front() else {
                return Ok(());
            };
            self.deliver(envelope)?;
        }
        Err(ActorError::StepBudget(self.max_steps))
    }

    /// Host tasks waiting to be run, in spawn order.
    pub fn take_tasks(&mut self) -> Vec<ActorTask> {
        std::mem::take(&mut self.tasks)
    }

    /// Report that an effect actor finished, then stop it.
    ///
    /// The parent receives `xstate.done.actor.<id>` with `output` as payload.
    ///
    /// # Errors
    ///
    /// Returns [`ActorError::Unknown`] when the actor does not exist.
    pub fn complete(&mut self, id: &ActorId, output: serde_json::Value) -> Result<(), ActorError> {
        self.finish(id, format!("xstate.done.actor.{id}"), output)
    }

    /// Report that an effect actor failed, then stop it.
    ///
    /// The parent receives `xstate.error.actor.<id>` with
    /// `{"error": message}` as payload.
    ///
    /// # Errors
    ///
    /// Returns [`ActorError::Unknown`] when the actor does not exist.
    pub fn fail(&mut self, id: &ActorId, message: impl Into<String>) -> Result<(), ActorError> {
        self.finish(
            id,
            format!("xstate.error.actor.{id}"),
            serde_json::json!({ "error": message.into() }),
        )
    }

    fn finish(
        &mut self,
        id: &ActorId,
        event_type: String,
        payload: serde_json::Value,
    ) -> Result<(), ActorError> {
        let Some(parent) = self.actors.get(id).and_then(|actor| actor.parent.clone()) else {
            return if self.stop(id) {
                Ok(())
            } else {
                Err(ActorError::Unknown(id.clone()))
            };
        };
        self.inbox.push_back(Envelope {
            target: parent,
            event: DynamicEvent::with_payload(event_type, payload),
        });
        self.stop(id);
        Ok(())
    }

    /// The root actor, when the system still has one.
    #[must_use]
    pub fn root(&self) -> Option<ActorId> {
        self.actors
            .values()
            .find(|actor| actor.parent.is_none())
            .map(|actor| actor.id.clone())
    }

    /// Whether an actor with this name exists.
    #[must_use]
    pub fn exists(&self, id: &ActorId) -> bool {
        self.actors.contains_key(id)
    }

    /// Every actor name, in ascending order.
    pub fn ids(&self) -> impl Iterator<Item = &ActorId> + '_ {
        self.actors.keys()
    }

    /// An actor's current value.
    #[must_use]
    pub fn state(&self, id: &ActorId) -> Option<&ActorState> {
        self.actors.get(id).map(|actor| &actor.state)
    }

    /// A machine actor's active configuration.
    #[must_use]
    pub fn configuration(&self, id: &ActorId) -> Option<&Configuration<String>> {
        self.state(id)?.configuration()
    }

    /// A machine actor's context.
    #[must_use]
    pub fn context(&self, id: &ActorId) -> Option<&crate::DynamicContext> {
        self.state(id)?.context()
    }

    /// A reducer actor's value.
    #[must_use]
    pub fn value(&self, id: &ActorId) -> Option<&serde_json::Value> {
        self.state(id)?.value()
    }

    /// An actor's parent.
    #[must_use]
    pub fn parent(&self, id: &ActorId) -> Option<&ActorId> {
        self.actors.get(id)?.parent.as_ref()
    }

    /// An actor's lifetime scope. Run host work inside it so stopping the
    /// actor interrupts that work.
    #[must_use]
    pub fn scope(&self, id: &ActorId) -> Option<Arc<Scope>> {
        self.actors.get(id).map(|actor| Arc::clone(&actor.scope))
    }

    /// Register a finalizer that runs when this actor stops.
    ///
    /// Returns `false` when the actor does not exist.
    pub fn on_stop(&self, id: &ActorId, finalizer: impl FnOnce() + Send + 'static) -> bool {
        match self.actors.get(id) {
            Some(actor) => {
                actor.scope.add_finalizer(finalizer);
                true
            }
            None => false,
        }
    }

    /// Finalizer defects collected from closed scopes.
    #[must_use]
    pub fn defects(&self) -> &[Defect] {
        &self.defects
    }

    fn add_actor(
        &mut self,
        id: ActorId,
        parent: Option<ActorId>,
        logic: ActorLogic,
        scope: Arc<Scope>,
    ) {
        let (state, directives) = match &logic {
            ActorLogic::Machine(machine) => {
                let outcome = machine.enter(machine.initial_context());
                (
                    ActorState::Machine(MachineState {
                        configuration: outcome.configuration,
                        history: outcome.history,
                        context: outcome.context,
                    }),
                    outcome.directives,
                )
            }
            ActorLogic::Transition { initial, .. } => {
                (ActorState::Transition(initial.clone()), Vec::new())
            }
            ActorLogic::Effect(effect) => {
                self.tasks.push(ActorTask {
                    actor: id.clone(),
                    effect: (effect.start)(),
                });
                (ActorState::External, Vec::new())
            }
            ActorLogic::Stream(_) => (ActorState::External, Vec::new()),
            ActorLogic::Callback(callback) => {
                (callback.start)(CallbackSender {
                    actor: id.clone(),
                    tx: self.callback_tx.clone(),
                });
                (ActorState::External, Vec::new())
            }
        };
        self.actors.insert(
            id.clone(),
            Actor {
                id: id.clone(),
                parent,
                logic,
                state,
                scope,
            },
        );
        self.pending.extend(
            directives
                .into_iter()
                .map(|directive| (id.clone(), directive)),
        );
    }

    fn apply(&mut self, sender: &ActorId, directive: ActorDirective) -> Result<(), ActorError> {
        match directive {
            ActorDirective::Spawn { id, logic } => {
                if self.actors.contains_key(&id) {
                    return Err(ActorError::Duplicate(id));
                }
                let Some(parent) = self.actors.get(sender) else {
                    return Err(ActorError::Unknown(sender.clone()));
                };
                let scope = parent.scope.fork();
                self.add_actor(id, Some(sender.clone()), logic, scope);
                Ok(())
            }
            ActorDirective::Send { target, event } => {
                if !self.actors.contains_key(&target) {
                    return Err(ActorError::Unknown(target));
                }
                self.inbox.push_back(Envelope { target, event });
                Ok(())
            }
            ActorDirective::Stop { id } => {
                if self.stop(&id) {
                    Ok(())
                } else {
                    Err(ActorError::Unknown(id))
                }
            }
        }
    }

    fn deliver(&mut self, envelope: Envelope) -> Result<(), ActorError> {
        let Some(mut actor) = self.actors.remove(&envelope.target) else {
            return Err(ActorError::Unknown(envelope.target));
        };
        let mut directives = Vec::new();
        let mut done = false;
        match (&actor.logic, &mut actor.state) {
            (ActorLogic::Machine(machine), ActorState::Machine(state)) => {
                let outcome = machine.step_state(state, &envelope.event);
                *state = MachineState {
                    configuration: outcome.configuration,
                    history: outcome.history,
                    context: outcome.context,
                };
                directives = outcome.directives;
                done = outcome.done;
            }
            (ActorLogic::Transition { reduce, .. }, ActorState::Transition(value)) => {
                *value = reduce(value, &envelope.event);
            }
            _ => {}
        }
        let id = actor.id.clone();
        let parent = actor.parent.clone();
        self.actors.insert(id.clone(), actor);
        for directive in directives {
            self.apply(&id, directive)?;
        }
        if done && let Some(parent) = parent {
            let event = DynamicEvent::new(format!("xstate.done.actor.{id}"));
            self.inbox.push_back(Envelope {
                target: parent,
                event,
            });
        }
        Ok(())
    }

    /// Forward one value per stream actor into its parent's inbox.
    fn pump_streams(&mut self) -> bool {
        let mut events = Vec::new();
        let mut ended = Vec::new();
        for actor in self.actors.values() {
            if let ActorLogic::Stream(stream) = &actor.logic {
                let Ok(receiver) = stream.receiver.lock() else {
                    continue;
                };
                match receiver.try_recv() {
                    Ok(event) => {
                        let target = actor.parent.clone().unwrap_or_else(|| actor.id.clone());
                        events.push(Envelope { target, event });
                    }
                    Err(TryRecvError::Disconnected) => ended.push(actor.id.clone()),
                    Err(TryRecvError::Empty) => {}
                }
            }
        }
        let pumped = !events.is_empty() || !ended.is_empty();
        for envelope in events {
            self.inbox.push_back(envelope);
        }
        for id in ended {
            self.stop(&id);
        }
        pumped
    }

    fn pump_callbacks(&mut self) -> bool {
        let mut pumped = false;
        while let Ok((id, event)) = self.callback_rx.try_recv() {
            pumped = true;
            let Some(actor) = self.actors.get(&id) else {
                continue;
            };
            let target = actor.parent.clone().unwrap_or_else(|| id.clone());
            self.inbox.push_back(Envelope { target, event });
        }
        pumped
    }

    fn subtree(&self, root: &ActorId) -> Vec<ActorId> {
        let mut found = Vec::new();
        let mut queue = VecDeque::from([root.clone()]);
        while let Some(id) = queue.pop_front() {
            found.push(id.clone());
            for actor in self.actors.values() {
                if actor.parent.as_ref() == Some(&id) {
                    queue.push_back(actor.id.clone());
                }
            }
        }
        found
    }

    fn depth(&self, id: &ActorId) -> usize {
        let mut depth = 0;
        let mut cursor = self.actors.get(id).and_then(|actor| actor.parent.clone());
        while let Some(parent) = cursor {
            depth += 1;
            cursor = self
                .actors
                .get(&parent)
                .and_then(|actor| actor.parent.clone());
        }
        depth
    }
}

impl fmt::Debug for ActorSystem {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ActorSystem")
            .field("actors", &self.ids().collect::<Vec<_>>())
            .field("inbox", &self.inbox.len())
            .field("pending", &self.pending.len())
            .field("tasks", &self.tasks.len())
            .finish()
    }
}
