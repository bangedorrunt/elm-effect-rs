use elm_effect::Duration;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use crate::action::ActionQueue;
use crate::actor::ActorLogic;
use crate::machine::{Machine, MachineBuilder};
use crate::state_node::StateNode;
use crate::transition::Transition;
use crate::types::{MachineState, StepOutcome, TimerFired, TimerIntent};

/// Dynamic JSON-compatible event for schema-driven statecharts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DynamicEvent {
    /// Event type discriminant matching transition keys.
    pub r#type: String,
    /// Associated event payload.
    #[serde(default)]
    pub payload: serde_json::Value,
}

impl DynamicEvent {
    /// Create a new event with a type discriminant and null payload.
    pub fn new(event_type: impl Into<String>) -> Self {
        Self {
            r#type: event_type.into(),
            payload: serde_json::Value::Null,
        }
    }

    /// Create an event with a type discriminant and structured JSON payload.
    pub fn with_payload(event_type: impl Into<String>, payload: serde_json::Value) -> Self {
        Self {
            r#type: event_type.into(),
            payload,
        }
    }
}

/// Dynamic extended context for schema-driven statecharts.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DynamicContext {
    /// Quantitative state data.
    #[serde(default)]
    pub data: serde_json::Value,
    /// Executed lifecycle and transition action trace.
    #[serde(default)]
    pub log: Vec<String>,
}

impl DynamicContext {
    /// Create dynamic context from an initial JSON data value.
    pub fn from_data(data: serde_json::Value) -> Self {
        Self {
            data,
            log: Vec::new(),
        }
    }
}

/// Action list deserializer supporting single string or string array representations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ActionList {
    /// Single action identifier.
    Single(String),
    /// Sequence of action identifiers.
    Multiple(Vec<String>),
}

impl ActionList {
    /// Flatten into a vector of action identifiers.
    pub fn into_vec(self) -> Vec<String> {
        match self {
            Self::Single(s) => vec![s],
            Self::Multiple(v) => v,
        }
    }
}

impl Default for ActionList {
    fn default() -> Self {
        Self::Multiple(Vec::new())
    }
}

/// A single target identifier or an XState target array.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TargetList {
    /// One target state.
    Single(String),
    /// Several target states entered by one transition.
    Multiple(Vec<String>),
}

impl TargetList {
    /// Flatten into target identifiers in declaration order.
    pub fn into_vec(self) -> Vec<String> {
        match self {
            Self::Single(target) => vec![target],
            Self::Multiple(targets) => targets,
        }
    }
}

/// One invoked actor declaration. The actor starts on state entry and stops on
/// state exit; its completion event is `xstate.done.actor.<id>`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvokeConfig {
    /// Stable child id. Defaults to `src`.
    #[serde(default)]
    pub id: Option<String>,
    /// Registered actor logic name.
    pub src: String,
    /// Input passed to the invoked actor when this state is entered.
    #[serde(default)]
    pub input: Option<serde_json::Value>,
    /// Transition taken when the invoked actor completes.
    #[serde(rename = "onDone", default)]
    pub on_done: Option<TransitionRule>,
    /// Transition taken when the invoked actor fails.
    #[serde(rename = "onError", default)]
    pub on_error: Option<TransitionRule>,
}

/// One invoke declaration or a declaration list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum InvokeList {
    /// One invoked actor.
    Single(Box<InvokeConfig>),
    /// Several invoked actors.
    Multiple(Vec<InvokeConfig>),
}

impl InvokeList {
    /// Flatten into invoke declarations in order.
    pub fn into_vec(self) -> Vec<InvokeConfig> {
        match self {
            Self::Single(invoke) => vec![*invoke],
            Self::Multiple(invokes) => invokes,
        }
    }
}

impl Default for InvokeList {
    fn default() -> Self {
        Self::Multiple(Vec::new())
    }
}

/// A schema delay source: a millisecond value or another named delay.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum DelaySpec {
    /// A literal millisecond delay.
    Millis(u64),
    /// A delay expression naming another entry, or a duration string.
    Reference(String),
}

/// Declarative transition target configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransitionConfig {
    /// Target state identifier or identifiers. Omit it for a targetless
    /// transition that only runs actions.
    #[serde(default)]
    pub target: Option<TargetList>,
    /// Shallow event payload fields that must match for this transition.
    #[serde(default)]
    pub matches: Option<serde_json::Value>,
    /// Optional conditional guard name.
    #[serde(default)]
    pub guard: Option<String>,
    /// Whether a target equal to the source, or inside it, exits and re-enters
    /// the source. XState defaults to `false`.
    #[serde(default)]
    pub reenter: bool,
    /// Optional list of actions to execute during transition.
    #[serde(default)]
    pub actions: ActionList,
}

/// Declarative transition rule supporting shorthand string target, single config, or multiple guarded targets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TransitionRule {
    /// Shorthand string target with unconditional transition and no actions.
    Shorthand(String),
    /// Single transition configuration.
    Single(TransitionConfig),
    /// Multiple transition configurations evaluated in declaration order.
    Multiple(Vec<TransitionConfig>),
}

impl TransitionRule {
    /// Expand into a list of transition configurations.
    pub fn into_configs(self) -> Vec<TransitionConfig> {
        match self {
            Self::Shorthand(target) => vec![TransitionConfig {
                target: Some(TargetList::Single(target)),
                matches: None,
                guard: None,
                reenter: false,
                actions: ActionList::default(),
            }],
            Self::Single(cfg) => vec![cfg],
            Self::Multiple(list) => list,
        }
    }
}

/// Declarative state node schema definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateNodeConfig {
    /// Node type: "atomic", "compound", "parallel", "history", or "final".
    #[serde(default)]
    pub r#type: Option<String>,
    /// Initial child state when node is compound, and the fallback target of a
    /// history node when nothing is remembered.
    #[serde(default)]
    pub initial: Option<String>,
    /// Default target or targets of a history node. Takes precedence over
    /// `initial` when both are present.
    #[serde(default)]
    pub target: Option<TargetList>,
    /// Stable identifier for `#id` transition targets.
    #[serde(default)]
    pub id: Option<String>,
    /// Parent compound or parallel state identifier.
    #[serde(default)]
    pub parent: Option<String>,
    /// History depth of a `"history"` node: `"shallow"` (default) or `"deep"`.
    #[serde(default)]
    pub history: Option<String>,
    /// Actions executed upon entering this state.
    #[serde(default)]
    pub entry: ActionList,
    /// Actions executed upon exiting this state.
    #[serde(default)]
    pub exit: ActionList,
    /// Event transitions mapped by event type.
    #[serde(default)]
    pub on: BTreeMap<String, TransitionRule>,
    /// Delayed transitions mapped by a millisecond delay.
    #[serde(default)]
    pub after: BTreeMap<String, TransitionRule>,
    /// Eventless (transient) transitions evaluated upon entering this state.
    #[serde(default)]
    pub always: Vec<TransitionConfig>,
    /// Transition taken when this node completes.
    #[serde(rename = "onDone", default)]
    pub on_done: Option<TransitionRule>,
    /// Output carried by this final state's completion event.
    #[serde(default)]
    pub output: Option<serde_json::Value>,
    /// Actors owned by this state's lifetime.
    #[serde(default)]
    pub invoke: InvokeList,
}

/// Root declarative statechart schema definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MachineConfig {
    /// Optional machine identifier.
    #[serde(default)]
    pub id: Option<String>,
    /// Root node type. Only `"parallel"` changes the implicit root from a
    /// compound node with one initial child to concurrent top-level regions.
    #[serde(default)]
    pub r#type: Option<String>,
    /// Root initial state identifier.
    pub initial: String,
    /// Root initial extended context data.
    #[serde(default)]
    pub context: serde_json::Value,
    /// Actions executed when the implicit machine root is entered.
    #[serde(default)]
    pub entry: ActionList,
    /// Actions executed when the implicit machine root is exited after
    /// completion.
    #[serde(default)]
    pub exit: ActionList,
    /// Named delay sources referenced by `after` keys.
    #[serde(default)]
    pub delays: BTreeMap<String, DelaySpec>,
    /// Output carried when the root completes. A topological final state's
    /// `output` is used when this is absent.
    #[serde(default)]
    pub output: Option<serde_json::Value>,
    /// Map of state node configurations.
    #[serde(default)]
    pub states: BTreeMap<String, StateNodeConfig>,
}

/// Function type for dynamic guard evaluations.
pub type DynamicGuardFn =
    Arc<dyn Fn(&DynamicContext, &DynamicEvent) -> bool + Send + Sync + 'static>;

/// Function type for dynamic action handlers.
pub type DynamicActionFn = Arc<
    dyn Fn(&mut DynamicContext, &DynamicEvent, &mut ActionQueue<String, DynamicEvent>)
        + Send
        + Sync
        + 'static,
>;

/// Function type for a named delay resolved from context and the entry event.
pub type DynamicDelayFn =
    Arc<dyn Fn(&DynamicContext, &DynamicEvent) -> Option<Duration> + Send + Sync + 'static>;

/// Function type for invoke input resolved at state entry.
pub type DynamicInputFn =
    Arc<dyn Fn(&DynamicContext, &DynamicEvent) -> serde_json::Value + Send + Sync + 'static>;

/// Function type for machine or final-state completion output.
pub type DynamicOutputFn =
    Arc<dyn Fn(&DynamicContext, &DynamicEvent) -> serde_json::Value + Send + Sync + 'static>;

/// A compiled schema-driven statechart machine operating on dynamic JSON values.
#[derive(Clone)]
pub struct DynamicMachine {
    inner: Machine<String, DynamicEvent, DynamicContext, String>,
    outputs: BTreeMap<String, DynamicOutputFn>,
    root_output: Option<DynamicOutputFn>,
    top_level_final: BTreeSet<String>,
}

impl DynamicMachine {
    /// Create a dynamic machine builder initialized from a schema configuration.
    pub fn builder(config: MachineConfig) -> DynamicMachineBuilder {
        DynamicMachineBuilder::new(config)
    }

    /// Parse a JSON schema string and compile a dynamic machine with default handlers.
    ///
    /// # Errors
    ///
    /// Returns the parse error when the schema is not valid JSON or does not
    /// match [`MachineConfig`].
    ///
    /// # Panics
    ///
    /// Panics when the schema invokes an actor, because this entry point has no
    /// place to register actor logic. Use [`DynamicMachine::builder`] and
    /// [`DynamicMachineBuilder::try_build`] for schemas with `invoke`.
    pub fn from_json_str(json: &str) -> Result<Self, serde_json::Error> {
        let config: MachineConfig = serde_json::from_str(json)?;
        Ok(Self::builder(config).build())
    }

    /// The configuration the machine starts in, with compound and parallel
    /// nodes expanded to their leaves.
    #[must_use]
    pub fn initial_configuration(&self) -> crate::Configuration<String> {
        self.inner.initial_configuration()
    }

    /// Enter the initial configuration, running entry actions and arming timers.
    pub fn enter(
        &self,
        context: &DynamicContext,
    ) -> StepOutcome<String, DynamicContext, String, DynamicEvent> {
        self.inner
            .enter_with_event(context, &DynamicEvent::new("xstate.init"))
    }

    /// Enter the initial configuration and settle its startup macrostep.
    pub fn start(
        &self,
        context: &DynamicContext,
        init: &DynamicEvent,
    ) -> StepOutcome<String, DynamicContext, String, DynamicEvent> {
        self.inner.start(context, init)
    }

    /// Step a full run state, including recorded history.
    pub fn step_state(
        &self,
        state: &MachineState<String, DynamicContext>,
        event: &DynamicEvent,
    ) -> StepOutcome<String, DynamicContext, String, DynamicEvent> {
        self.inner.step_state(state, event)
    }

    /// Step the statechart with an incoming dynamic event.
    pub fn step(
        &self,
        current_state: &str,
        context: &DynamicContext,
        event: &DynamicEvent,
    ) -> StepOutcome<String, DynamicContext, String, DynamicEvent> {
        let state = MachineState::new(
            self.inner.resolve_configuration(&current_state.to_string()),
            context.clone(),
        );
        self.inner.step_state(&state, event)
    }

    /// Take an elapsed timer for a dynamic machine.
    pub fn step_timer(
        &self,
        state: &MachineState<String, DynamicContext>,
        timer: &TimerFired<String>,
        event: &DynamicEvent,
    ) -> StepOutcome<String, DynamicContext, String, DynamicEvent> {
        self.inner.step_timer(state, timer, event)
    }

    /// Access the initial root state.
    #[must_use]
    pub fn initial_state(&self) -> &str {
        self.inner.initial_state()
    }

    /// Access the initial extended context.
    #[must_use]
    pub fn initial_context(&self) -> &DynamicContext {
        self.inner.initial_context()
    }

    /// Resolve a state to its active leaf node.
    #[must_use]
    pub fn resolve_leaf(&self, state: &str) -> String {
        self.inner.resolve_leaf(&state.to_string())
    }

    /// Timer intents for the delays of every active state.
    #[must_use]
    pub fn timers_for_state(
        &self,
        state: &MachineState<String, DynamicContext>,
    ) -> Vec<TimerIntent<String>> {
        self.inner.timers_for_state(state)
    }

    /// Rebuild timer bookkeeping after restoring a snapshot.
    pub fn rearm_timers(&self, state: &mut MachineState<String, DynamicContext>) {
        self.inner.rearm_timers(state);
    }

    /// The output carried by the completion event for this run, when the root
    /// has completed. A root output wins; otherwise the active top-level final
    /// state's output is used.
    #[must_use]
    pub fn completion_output(
        &self,
        state: &MachineState<String, DynamicContext>,
        event: &DynamicEvent,
    ) -> Option<serde_json::Value> {
        if let Some(resolve) = &self.root_output {
            return Some(resolve(&state.context, event));
        }
        state
            .configuration
            .leaves()
            .find(|leaf| self.top_level_final.contains(*leaf))
            .and_then(|leaf| self.outputs.get(leaf))
            .map(|resolve| resolve(&state.context, event))
    }

    /// Render the compiled chart as a Mermaid diagram.
    #[must_use]
    pub fn to_mermaid(&self) -> String {
        self.inner.to_mermaid()
    }
}

/// Why a dynamic machine cannot be built.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DynamicBuildError {
    /// An invoke names an actor source the builder has not registered.
    #[error("state `{state}` invokes unknown actor source `{src}`")]
    UnknownInvokeSource {
        /// State owning the invoke.
        state: String,
        /// Unregistered `src`.
        src: String,
    },
    /// One state declares the same invoke id more than once.
    #[error("state `{state}` declares invoke id `{id}` more than once")]
    DuplicateInvokeId {
        /// State owning the invokes.
        state: String,
        /// Repeated child id.
        id: String,
    },
}

/// Builder for compiling a [`MachineConfig`] into an executable [`DynamicMachine`].
pub struct DynamicMachineBuilder {
    config: MachineConfig,
    guards: BTreeMap<String, DynamicGuardFn>,
    actions: BTreeMap<String, DynamicActionFn>,
    delays: BTreeMap<String, DynamicDelayFn>,
    actors: BTreeMap<String, ActorLogic>,
    invoke_inputs: BTreeMap<String, DynamicInputFn>,
    outputs: BTreeMap<String, DynamicOutputFn>,
    root_output: Option<DynamicOutputFn>,
}

impl DynamicMachineBuilder {
    /// Initialize builder with machine schema configuration.
    pub fn new(config: MachineConfig) -> Self {
        Self {
            config,
            guards: BTreeMap::new(),
            actions: BTreeMap::new(),
            delays: BTreeMap::new(),
            actors: BTreeMap::new(),
            invoke_inputs: BTreeMap::new(),
            outputs: BTreeMap::new(),
            root_output: None,
        }
    }

    /// Register a named guard condition.
    pub fn guard(
        mut self,
        name: impl Into<String>,
        predicate: impl Fn(&DynamicContext, &DynamicEvent) -> bool + Send + Sync + 'static,
    ) -> Self {
        self.guards.insert(name.into(), Arc::new(predicate));
        self
    }

    /// Register a named delay function. An `after` key selects it by name and
    /// the function recomputes the duration from context and the entering event
    /// each time the state is entered.
    pub fn delay(
        mut self,
        name: impl Into<String>,
        resolve: impl Fn(&DynamicContext, &DynamicEvent) -> Option<Duration> + Send + Sync + 'static,
    ) -> Self {
        self.delays.insert(name.into(), Arc::new(resolve));
        self
    }

    /// Register actor logic addressable by an `invoke` `src`.
    pub fn actor(mut self, name: impl Into<String>, logic: ActorLogic) -> Self {
        self.actors.insert(name.into(), logic);
        self
    }

    /// Resolve an invoked actor's input from context and the event that entered
    /// the owning state. The key is the invoke id, or its `src` when no id is
    /// declared.
    pub fn invoke_input(
        mut self,
        id: impl Into<String>,
        resolve: impl Fn(&DynamicContext, &DynamicEvent) -> serde_json::Value + Send + Sync + 'static,
    ) -> Self {
        self.invoke_inputs.insert(id.into(), Arc::new(resolve));
        self
    }

    /// Resolve a final state's completion output from context and the event
    /// that completed the machine.
    pub fn output(
        mut self,
        state: impl Into<String>,
        resolve: impl Fn(&DynamicContext, &DynamicEvent) -> serde_json::Value + Send + Sync + 'static,
    ) -> Self {
        self.outputs.insert(state.into(), Arc::new(resolve));
        self
    }

    /// Resolve the machine root's completion output from context and the event
    /// that completed it.
    pub fn root_output(
        mut self,
        resolve: impl Fn(&DynamicContext, &DynamicEvent) -> serde_json::Value + Send + Sync + 'static,
    ) -> Self {
        self.root_output = Some(Arc::new(resolve));
        self
    }

    /// Register a named action handler.
    pub fn action(
        mut self,
        name: impl Into<String>,
        handler: impl Fn(&mut DynamicContext, &DynamicEvent, &mut ActionQueue<String, DynamicEvent>)
        + Send
        + Sync
        + 'static,
    ) -> Self {
        self.actions.insert(name.into(), Arc::new(handler));
        self
    }

    fn build_action_fn(
        action_name: String,
        custom: &BTreeMap<String, DynamicActionFn>,
    ) -> DynamicActionFn {
        if let Some(handler) = custom.get(&action_name) {
            return Arc::clone(handler);
        }

        let name = action_name.clone();
        Arc::new(
            move |ctx: &mut DynamicContext,
                  _event: &DynamicEvent,
                  _enq: &mut ActionQueue<String, DynamicEvent>| {
                if let Some(assign_expr) = name.strip_prefix("assign:") {
                    Self::apply_assignment(&mut ctx.data, assign_expr);
                } else {
                    ctx.log.push(name.clone());
                }
            },
        )
    }

    fn apply_assignment(data: &mut serde_json::Value, expr: &str) {
        if let Some((key, delta)) = expr.split_once("+=") {
            let key = key.trim();
            if let Ok(delta_val) = delta.trim().parse::<i64>()
                && let serde_json::Value::Object(map) = data
            {
                let curr = map
                    .get(key)
                    .and_then(serde_json::Value::as_i64)
                    .unwrap_or(0);
                map.insert(key.to_string(), serde_json::Value::from(curr + delta_val));
            }
        } else if let Some((key, val_str)) = expr.split_once('=') {
            let key = key.trim();
            let val_str = val_str.trim();
            let val = if let Ok(n) = val_str.parse::<i64>() {
                serde_json::Value::from(n)
            } else if let Ok(b) = val_str.parse::<bool>() {
                serde_json::Value::from(b)
            } else {
                serde_json::Value::from(val_str)
            };
            if let serde_json::Value::Object(map) = data {
                map.insert(key.to_string(), val);
            }
        }
    }

    fn build_guard_fn(
        guard_name: String,
        custom: &BTreeMap<String, DynamicGuardFn>,
    ) -> DynamicGuardFn {
        if let Some(predicate) = custom.get(&guard_name) {
            return Arc::clone(predicate);
        }

        let name = guard_name.clone();
        Arc::new(move |ctx: &DynamicContext, _event: &DynamicEvent| {
            if name == "true" {
                return true;
            }
            if name == "false" {
                return false;
            }
            if let Some((field, threshold)) = name.split_once("<=") {
                return Self::compare_field(&ctx.data, field.trim(), threshold.trim(), |a, b| {
                    a <= b
                });
            }
            if let Some((field, threshold)) = name.split_once(">=") {
                return Self::compare_field(&ctx.data, field.trim(), threshold.trim(), |a, b| {
                    a >= b
                });
            }
            if let Some((field, threshold)) = name.split_once('<') {
                return Self::compare_field(&ctx.data, field.trim(), threshold.trim(), |a, b| {
                    a < b
                });
            }
            if let Some((field, threshold)) = name.split_once('>') {
                return Self::compare_field(&ctx.data, field.trim(), threshold.trim(), |a, b| {
                    a > b
                });
            }
            if let Some((field, threshold)) = name.split_once("==") {
                return Self::compare_field(&ctx.data, field.trim(), threshold.trim(), |a, b| {
                    a == b
                });
            }
            true
        })
    }

    fn compare_field(
        data: &serde_json::Value,
        field: &str,
        threshold: &str,
        op: impl Fn(i64, i64) -> bool,
    ) -> bool {
        let Some(curr) = data.get(field).and_then(serde_json::Value::as_i64) else {
            return false;
        };
        let Ok(limit) = threshold.parse::<i64>() else {
            return false;
        };
        op(curr, limit)
    }

    fn resolve_named_delay(
        name: &str,
        custom: &BTreeMap<String, DynamicDelayFn>,
        specs: &BTreeMap<String, DelaySpec>,
        context: &DynamicContext,
        event: &DynamicEvent,
    ) -> Option<Duration> {
        let mut cursor = name;
        let mut seen = BTreeSet::new();
        while seen.insert(cursor.to_string()) {
            if let Some(resolve) = custom.get(cursor) {
                return resolve(context, event);
            }
            match specs.get(cursor)? {
                DelaySpec::Millis(millis) => return Some(Duration::from_millis(*millis)),
                DelaySpec::Reference(reference) => {
                    if let Some(millis) = Self::parse_delay_ms(reference) {
                        return Some(Duration::from_millis(millis));
                    }
                    cursor = reference;
                }
            }
        }
        None
    }

    fn parse_delay_ms(raw: &str) -> Option<u64> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return None;
        }
        if let Ok(millis) = trimmed.parse::<u64>() {
            return Some(millis);
        }
        if let Some(iso) = trimmed
            .strip_prefix("PT")
            .or_else(|| trimmed.strip_prefix("pt"))
        {
            let mut total = 0.0;
            let mut number = String::new();
            for ch in iso.chars() {
                if ch.is_ascii_digit() || ch == '.' {
                    number.push(ch);
                    continue;
                }
                let value = number.parse::<f64>().ok()?;
                let factor = match ch.to_ascii_uppercase() {
                    'H' => 3_600_000.0,
                    'M' => 60_000.0,
                    'S' => 1_000.0,
                    _ => return None,
                };
                total += value * factor;
                number.clear();
            }
            if !number.is_empty() || !total.is_finite() || total < 0.0 {
                return None;
            }
            return Some(total.round() as u64);
        }
        let lower = trimmed.to_ascii_lowercase();
        for (suffix, factor) in [
            ("ms", 1.0),
            ("s", 1_000.0),
            ("m", 60_000.0),
            ("h", 3_600_000.0),
            ("d", 86_400_000.0),
        ] {
            let Some(value) = lower.strip_suffix(suffix) else {
                continue;
            };
            let number = value.parse::<f64>().ok()?;
            if number.is_finite() && number >= 0.0 {
                return Some((number * factor).round() as u64);
            }
        }
        None
    }

    fn payload_matches(matches: Option<&serde_json::Value>, event: &DynamicEvent) -> bool {
        let Some(serde_json::Value::Object(fields)) = matches else {
            return matches.is_none();
        };
        fields
            .iter()
            .all(|(key, expected)| event.payload.get(key) == Some(expected))
    }

    fn descriptor_matches(descriptor: &str, event_type: &str) -> bool {
        if descriptor == "*" {
            return true;
        }
        if let Some(prefix) = descriptor.strip_suffix(".*") {
            return event_type == prefix
                || event_type
                    .strip_prefix(prefix)
                    .is_some_and(|rest| rest.starts_with('.'));
        }
        descriptor == event_type
    }

    fn descriptor_priority(descriptor: &str, event_type: &str) -> (u8, usize) {
        if descriptor == event_type {
            return (1, 0);
        }
        if let Some(prefix) = descriptor.strip_suffix(".*") {
            return (2, usize::MAX - prefix.len());
        }
        (3, 0)
    }

    fn state_paths(config: &MachineConfig) -> BTreeMap<String, String> {
        let mut paths = BTreeMap::new();
        for state_id in config.states.keys() {
            let mut segments = Vec::new();
            let mut seen = BTreeSet::new();
            let mut cursor = Some(state_id.as_str());
            while let Some(current) = cursor {
                if !seen.insert(current.to_string()) {
                    break;
                }
                segments.push(current.to_string());
                cursor = config
                    .states
                    .get(current)
                    .and_then(|node| node.parent.as_deref());
            }
            segments.reverse();
            paths.insert(state_id.clone(), segments.join("."));
        }
        paths
    }

    fn state_ids(config: &MachineConfig) -> BTreeMap<String, String> {
        let mut ids = BTreeMap::new();
        for (state_id, node) in &config.states {
            ids.insert(state_id.clone(), state_id.clone());
            if let Some(id) = &node.id {
                ids.insert(id.clone(), state_id.clone());
            }
        }
        ids
    }

    fn resolve_target(
        source: &str,
        raw: &str,
        paths: &BTreeMap<String, String>,
        ids: &BTreeMap<String, String>,
    ) -> String {
        let raw = raw.trim();
        if let Some(id) = raw.strip_prefix('#') {
            return ids.get(id).cloned().unwrap_or_else(|| id.to_string());
        }
        let source_path = paths
            .get(source)
            .cloned()
            .unwrap_or_else(|| source.to_string());
        let candidate = if let Some(relative) = raw.strip_prefix('.') {
            format!("{source_path}.{}", relative.trim_start_matches('.'))
        } else {
            match source_path.rsplit_once('.') {
                Some((parent, _)) => format!("{parent}.{raw}"),
                None => raw.to_string(),
            }
        };
        if let Some((key, _)) = paths.iter().find(|(_, path)| path.as_str() == candidate) {
            return key.clone();
        }
        if paths.contains_key(raw) {
            return raw.to_string();
        }
        if let Some((key, _)) = paths.iter().find(|(_, path)| path.as_str() == raw) {
            return key.clone();
        }
        raw.to_string()
    }

    fn transition_for(
        source: &str,
        config: &TransitionConfig,
        paths: &BTreeMap<String, String>,
        ids: &BTreeMap<String, String>,
    ) -> Transition<String, DynamicEvent, DynamicContext, String> {
        match &config.target {
            Some(target) => {
                let mut targets: Vec<String> = target
                    .clone()
                    .into_vec()
                    .into_iter()
                    .map(|raw| Self::resolve_target(source, &raw, paths, ids))
                    .collect();
                let transition = if targets.len() == 1 {
                    Transition::target(targets.remove(0))
                } else {
                    Transition::target(targets[0].clone()).targets(targets)
                };
                transition.reenter_with(config.reenter)
            }
            None => Transition::target(source.to_string()).targetless(),
        }
    }

    fn descendants_of(config: &MachineConfig, state: &str) -> BTreeSet<String> {
        let mut descendants = BTreeSet::new();
        let mut frontier = vec![state.to_string()];
        while let Some(parent) = frontier.pop() {
            for (id, node) in &config.states {
                if node.parent.as_deref() == Some(parent.as_str()) && descendants.insert(id.clone())
                {
                    frontier.push(id.clone());
                }
            }
        }
        descendants
    }

    /// Attach the schema's guards and actions to one transition.
    fn decorate(
        &self,
        mut transition: Transition<String, DynamicEvent, DynamicContext, String>,
        config: &TransitionConfig,
    ) -> Transition<String, DynamicEvent, DynamicContext, String> {
        if let Some(guard_name) = &config.guard {
            if let Some(state_path) = guard_name.strip_prefix("state_in:") {
                let target = state_path
                    .rsplit('.')
                    .next()
                    .unwrap_or(state_path)
                    .to_string();
                let descendants = Self::descendants_of(&self.config, &target);
                transition = transition.configuration_guard(move |configuration| {
                    configuration
                        .leaves()
                        .any(|leaf| leaf == &target || descendants.contains(leaf))
                });
            } else {
                let guard_fn = Self::build_guard_fn(guard_name.clone(), &self.guards);
                transition = transition.guard(move |ctx, e| guard_fn(ctx, e));
            }
        }
        for action_name in config.actions.clone().into_vec() {
            let action_fn = Self::build_action_fn(action_name, &self.actions);
            transition = transition.action(move |ctx, e, enq| action_fn(ctx, e, enq));
        }
        transition
    }

    fn validate_invokes(&self) -> Result<(), DynamicBuildError> {
        for (state, config) in &self.config.states {
            let mut ids = BTreeSet::new();
            for invoke in config.invoke.clone().into_vec() {
                if !self.actors.contains_key(&invoke.src) {
                    return Err(DynamicBuildError::UnknownInvokeSource {
                        state: state.clone(),
                        src: invoke.src,
                    });
                }
                let id = invoke.id.unwrap_or(invoke.src);
                if !ids.insert(id.clone()) {
                    return Err(DynamicBuildError::DuplicateInvokeId {
                        state: state.clone(),
                        id,
                    });
                }
            }
        }
        Ok(())
    }

    /// Compile, rejecting invoke configurations that cannot run.
    ///
    /// # Errors
    ///
    /// Returns [`DynamicBuildError::UnknownInvokeSource`] when an invoke names
    /// an unregistered source, or [`DynamicBuildError::DuplicateInvokeId`] when
    /// one state declares the same id twice.
    pub fn try_build(self) -> Result<DynamicMachine, DynamicBuildError> {
        self.validate_invokes()?;
        Ok(self.build_validated())
    }

    /// Compile into an immutable [`DynamicMachine`].
    ///
    /// # Panics
    ///
    /// Panics when [`try_build`](Self::try_build) would return an error.
    #[must_use]
    pub fn build(self) -> DynamicMachine {
        self.try_build().unwrap_or_else(|error| panic!("{error}"))
    }

    fn build_validated(self) -> DynamicMachine {
        let initial_context = DynamicContext::from_data(self.config.context.clone());
        let paths = Self::state_paths(&self.config);
        let ids = Self::state_ids(&self.config);
        let mut builder: MachineBuilder<String, DynamicEvent, DynamicContext, String> =
            Machine::builder(self.config.initial.clone(), initial_context);
        if self.config.r#type.as_deref() == Some("parallel") {
            builder = builder.root_parallel();
        }

        for entry_name in self.config.entry.clone().into_vec() {
            let action_fn = Self::build_action_fn(entry_name, &self.actions);
            builder = builder.root_entry(move |ctx, enq| {
                let dummy_event = DynamicEvent::new("entry");
                action_fn(ctx, &dummy_event, enq);
            });
        }

        for exit_name in self.config.exit.clone().into_vec() {
            let action_fn = Self::build_action_fn(exit_name, &self.actions);
            builder = builder.root_exit(move |ctx, enq| {
                let dummy_event = DynamicEvent::new("exit");
                action_fn(ctx, &dummy_event, enq);
            });
        }

        for (state_id, state_cfg) in &self.config.states {
            let is_history = state_cfg.r#type.as_deref() == Some("history");
            let is_parallel = state_cfg.r#type.as_deref() == Some("parallel");
            let is_final = state_cfg.r#type.as_deref() == Some("final");
            let is_compound =
                state_cfg.r#type.as_deref() == Some("compound") || state_cfg.initial.is_some();

            let mut node = if is_final {
                StateNode::final_state(state_id.clone())
            } else if is_history {
                let mut targets: Vec<String> = state_cfg
                    .target
                    .clone()
                    .map(TargetList::into_vec)
                    .unwrap_or_default()
                    .into_iter()
                    .map(|raw| Self::resolve_target(state_id, &raw, &paths, &ids))
                    .collect();
                if targets.is_empty() {
                    targets.push(
                        state_cfg
                            .initial
                            .clone()
                            .or_else(|| state_cfg.parent.clone())
                            .unwrap_or_else(|| state_id.clone()),
                    );
                }
                let primary = targets.first().cloned().unwrap_or_else(|| state_id.clone());
                StateNode::history(
                    state_id.clone(),
                    state_cfg.history.as_deref() == Some("deep"),
                    primary,
                )
                .with_initial_targets(targets)
            } else if is_parallel {
                StateNode::parallel(state_id.clone())
            } else if is_compound {
                let init = state_cfg
                    .initial
                    .clone()
                    .unwrap_or_else(|| state_id.clone());
                StateNode::compound(state_id.clone(), init)
            } else {
                StateNode::atomic(state_id.clone())
            };

            if let Some(ref parent) = state_cfg.parent {
                node = node.with_parent(parent.clone());
            }

            for invoke in state_cfg.invoke.clone().into_vec() {
                let actor_id = invoke.id.clone().unwrap_or_else(|| invoke.src.clone());
                if let Some(logic) = self.actors.get(&invoke.src).cloned() {
                    let spawn_id = actor_id.clone();
                    let static_input = invoke.input.clone();
                    let input_resolver = self.invoke_inputs.get(&actor_id).cloned();
                    node = node.entry_with_event(move |context, enq, event| {
                        let init = DynamicEvent::new("xstate.init");
                        let event = event.unwrap_or(&init);
                        let input = input_resolver.as_ref().map_or_else(
                            || static_input.clone().unwrap_or(serde_json::Value::Null),
                            |resolve| resolve(context, event),
                        );
                        enq.spawn_with_input(spawn_id.clone(), logic.clone(), input);
                    });
                    let stop_id = actor_id.clone();
                    node = node.exit(move |_ctx, enq| {
                        enq.stop(stop_id.clone());
                    });
                }
                if let Some(rule) = &invoke.on_done {
                    let done_event = format!("xstate.done.actor.{actor_id}");
                    for trans_cfg in rule.clone().into_configs() {
                        let matcher = done_event.clone();
                        let transition = Self::transition_for(state_id, &trans_cfg, &paths, &ids)
                            .on(move |event: &DynamicEvent| event.r#type == matcher)
                            .label(done_event.clone());
                        let transition = self.decorate(transition, &trans_cfg);
                        node = node.on(transition);
                    }
                }
                if let Some(rule) = &invoke.on_error {
                    let error_event = format!("xstate.error.actor.{actor_id}");
                    for trans_cfg in rule.clone().into_configs() {
                        let matcher = error_event.clone();
                        let transition = Self::transition_for(state_id, &trans_cfg, &paths, &ids)
                            .on(move |event: &DynamicEvent| event.r#type == matcher)
                            .label(error_event.clone());
                        let transition = self.decorate(transition, &trans_cfg);
                        node = node.on(transition);
                    }
                }
            }

            for entry_name in state_cfg.entry.clone().into_vec() {
                let action_fn = Self::build_action_fn(entry_name, &self.actions);
                node = node.entry(move |ctx, enq| {
                    let dummy_event = DynamicEvent::new("entry");
                    action_fn(ctx, &dummy_event, enq);
                });
            }

            for exit_name in state_cfg.exit.clone().into_vec() {
                let action_fn = Self::build_action_fn(exit_name, &self.actions);
                node = node.exit(move |ctx, enq| {
                    let dummy_event = DynamicEvent::new("exit");
                    action_fn(ctx, &dummy_event, enq);
                });
            }

            for (event_type, transition_rule) in &state_cfg.on {
                for trans_cfg in transition_rule.clone().into_configs() {
                    let event_matcher = event_type.clone();
                    let priority_descriptor = event_type.clone();
                    let payload_matcher = trans_cfg.matches.clone();
                    let transition = Self::transition_for(state_id, &trans_cfg, &paths, &ids)
                        .on(move |e: &DynamicEvent| {
                            Self::descriptor_matches(&event_matcher, &e.r#type)
                                && Self::payload_matches(payload_matcher.as_ref(), e)
                        })
                        .with_selection_priority(move |e: &DynamicEvent| {
                            Self::descriptor_priority(&priority_descriptor, &e.r#type)
                        })
                        .label(event_type.clone());
                    node = node.on(self.decorate(transition, &trans_cfg));
                }
            }

            for (delay_key, transition_rule) in &state_cfg.after {
                let timer_delay = Self::parse_delay_ms(delay_key)
                    .map(|millis| millis.to_string())
                    .unwrap_or_else(|| delay_key.clone());
                let timer_event = format!("xstate.after.{timer_delay}.{state_id}");
                for trans_cfg in transition_rule.clone().into_configs() {
                    let matcher = timer_event.clone();
                    let transition = Self::transition_for(state_id, &trans_cfg, &paths, &ids)
                        .on(move |e: &DynamicEvent| e.r#type == matcher)
                        .label(timer_event.clone());
                    let transition = self.decorate(transition, &trans_cfg);
                    node = if let Some(millis) = Self::parse_delay_ms(delay_key) {
                        node.after(Duration::from_millis(millis), transition)
                    } else {
                        let key = delay_key.clone();
                        let custom = self.delays.clone();
                        let specs = self.config.delays.clone();
                        node.after_event(
                            delay_key.clone(),
                            move |context, event| {
                                Self::resolve_named_delay(&key, &custom, &specs, context, event)
                            },
                            transition,
                        )
                    };
                }
            }

            for trans_cfg in &state_cfg.always {
                let transition = Self::transition_for(state_id, trans_cfg, &paths, &ids);
                node = node.always(self.decorate(transition, trans_cfg));
            }

            if let Some(rule) = &state_cfg.on_done {
                for trans_cfg in rule.clone().into_configs() {
                    let transition =
                        Self::transition_for(state_id, &trans_cfg, &paths, &ids).label("done");
                    node = node.on_done(self.decorate(transition, &trans_cfg));
                }
            }

            builder = builder.node(node);
        }

        let mut outputs: BTreeMap<String, DynamicOutputFn> = BTreeMap::new();
        for (state_id, state_cfg) in &self.config.states {
            if let Some(output) = state_cfg.output.clone() {
                outputs.insert(
                    state_id.clone(),
                    Arc::new(move |_context, _event| output.clone()),
                );
            }
        }
        outputs.extend(self.outputs);
        let root_output = self.root_output.or_else(|| {
            self.config.output.clone().map(|output| {
                Arc::new(move |_context: &DynamicContext, _event: &DynamicEvent| output.clone())
                    as DynamicOutputFn
            })
        });
        let top_level_final = self
            .config
            .states
            .iter()
            .filter(|(_, state_cfg)| {
                state_cfg.parent.is_none() && state_cfg.r#type.as_deref() == Some("final")
            })
            .map(|(state_id, _)| state_id.clone())
            .collect();

        DynamicMachine {
            inner: builder.build(),
            outputs,
            root_output,
            top_level_final,
        }
    }
}
