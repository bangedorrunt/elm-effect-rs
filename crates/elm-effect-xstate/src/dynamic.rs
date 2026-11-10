use elm_effect::Duration;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Arc;

use crate::action::ActionQueue;
use crate::machine::{Machine, MachineBuilder};
use crate::state_node::StateNode;
use crate::transition::Transition;
use crate::types::{MachineState, StepOutcome};

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

/// Declarative transition target configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransitionConfig {
    /// Target state identifier.
    pub target: String,
    /// Optional conditional guard name.
    #[serde(default)]
    pub guard: Option<String>,
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
                target,
                guard: None,
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
}

/// Root declarative statechart schema definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MachineConfig {
    /// Optional machine identifier.
    #[serde(default)]
    pub id: Option<String>,
    /// Root initial state identifier.
    pub initial: String,
    /// Root initial extended context data.
    #[serde(default)]
    pub context: serde_json::Value,
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

/// A compiled schema-driven statechart machine operating on dynamic JSON values.
#[derive(Clone)]
pub struct DynamicMachine {
    inner: Machine<String, DynamicEvent, DynamicContext, String>,
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
        self.inner.enter(context)
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

    /// Render the compiled chart as a Mermaid diagram.
    #[must_use]
    pub fn to_mermaid(&self) -> String {
        self.inner.to_mermaid()
    }
}

/// Builder for compiling a [`MachineConfig`] into an executable [`DynamicMachine`].
pub struct DynamicMachineBuilder {
    config: MachineConfig,
    guards: BTreeMap<String, DynamicGuardFn>,
    actions: BTreeMap<String, DynamicActionFn>,
}

impl DynamicMachineBuilder {
    /// Initialize builder with machine schema configuration.
    pub fn new(config: MachineConfig) -> Self {
        Self {
            config,
            guards: BTreeMap::new(),
            actions: BTreeMap::new(),
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

    /// Attach the schema's guards and actions to one transition.
    fn decorate(
        &self,
        mut transition: Transition<String, DynamicEvent, DynamicContext, String>,
        config: &TransitionConfig,
    ) -> Transition<String, DynamicEvent, DynamicContext, String> {
        if let Some(guard_name) = &config.guard {
            let guard_fn = Self::build_guard_fn(guard_name.clone(), &self.guards);
            transition = transition.guard(move |ctx, e| guard_fn(ctx, e));
        }
        for action_name in config.actions.clone().into_vec() {
            let action_fn = Self::build_action_fn(action_name, &self.actions);
            transition = transition.action(move |ctx, e, enq| action_fn(ctx, e, enq));
        }
        transition
    }

    /// Compile into an immutable [`DynamicMachine`].
    #[must_use]
    pub fn build(self) -> DynamicMachine {
        let initial_context = DynamicContext::from_data(self.config.context.clone());
        let mut builder: MachineBuilder<String, DynamicEvent, DynamicContext, String> =
            Machine::builder(self.config.initial.clone(), initial_context);

        for (state_id, state_cfg) in &self.config.states {
            let is_history = state_cfg.r#type.as_deref() == Some("history");
            let is_parallel = state_cfg.r#type.as_deref() == Some("parallel");
            let is_final = state_cfg.r#type.as_deref() == Some("final");
            let is_compound =
                state_cfg.r#type.as_deref() == Some("compound") || state_cfg.initial.is_some();

            let mut node = if is_final {
                StateNode::final_state(state_id.clone())
            } else if is_history {
                let fallback = state_cfg
                    .initial
                    .clone()
                    .or_else(|| state_cfg.parent.clone())
                    .unwrap_or_else(|| state_id.clone());
                StateNode::history(
                    state_id.clone(),
                    state_cfg.history.as_deref() == Some("deep"),
                    fallback,
                )
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
                    let transition = Transition::target(trans_cfg.target.clone())
                        .on(move |e: &DynamicEvent| e.r#type == event_matcher)
                        .label(event_type.clone());
                    node = node.on(self.decorate(transition, &trans_cfg));
                }
            }

            for (delay_ms, transition_rule) in &state_cfg.after {
                let Ok(millis) = delay_ms.trim().parse::<u64>() else {
                    continue;
                };
                let timer_event = format!("xstate.after.{millis}.{state_id}");
                for trans_cfg in transition_rule.clone().into_configs() {
                    let matcher = timer_event.clone();
                    let transition = Transition::target(trans_cfg.target.clone())
                        .on(move |e: &DynamicEvent| e.r#type == matcher)
                        .label(timer_event.clone());
                    node = node.after(
                        Duration::from_millis(millis),
                        self.decorate(transition, &trans_cfg),
                    );
                }
            }

            for trans_cfg in &state_cfg.always {
                let transition = Transition::target(trans_cfg.target.clone());
                node = node.always(self.decorate(transition, trans_cfg));
            }

            if let Some(rule) = &state_cfg.on_done {
                for trans_cfg in rule.clone().into_configs() {
                    let transition = Transition::target(trans_cfg.target.clone()).label("done");
                    node = node.on_done(self.decorate(transition, &trans_cfg));
                }
            }

            builder = builder.node(node);
        }

        DynamicMachine {
            inner: builder.build(),
        }
    }
}
