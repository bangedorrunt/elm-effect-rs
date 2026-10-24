//! Elm loop on Effect (ADR-0002 Phase 8, Law L12 structure).
//!
//! A [`Program`] is pure: `init` from parsed [`Boot`] config, `update`
//! folding `(Model, Msg)` into [`Update`] (new model plus commands).
//! Time enters as a message; the host samples the clock and sends `Msg`.
//! No live socket, clock, or store lives in here. [`Interpret`] turns
//! each command into an [`Effect`](elm_effect::Effect); the [`Loop`] runs
//! it on a [`Runtime`](elm_effect::Runtime) and feeds the outcome back as
//! a message. One entrance ([`Loop::send`]), one exit (the model).
//!
//! Every fold input is journaled before the fold sees it: host messages
//! and command outcomes alike. A durable command's intent is journaled
//! before its provider runs, and [`Ephemeral`](Durability::Ephemeral)
//! commands leave no trace. [`Loop::recover`] rebuilds the model from that
//! journal without running a provider. It runs durable commands that never
//! started, and it reports in-flight ones in [`Loop::in_flight`] for a
//! reconcile pass. [`Loop::snapshot`] bounds how much of the log a recovery
//! folds again.
//!
//! [`Subscriber`] reads the model and cannot write it: the handle exposes
//! no send path by construction (see `subscriber_cannot_write`). No
//! command outcome is lost: success keeps the interpreter's message, and
//! every failure, defect, and interrupt enters through
//! [`Program::failure`] as a [`Failure`]. A command window runs its
//! commands concurrently, because the runtime's run edge is stateless and
//! [`ManagedRuntime`] is `Sync`; the fold stays sequential, so a
//! concurrent window is still deterministic and ordered.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::rc::Rc;
use std::sync::Arc;

use elm_effect::{Cause, Defect, Effect, Exit, ManagedRuntime, Reason};
use elm_effect_journal::EffectState;
use elm_effect_journal::durable_log::{AppendError, DurableLog, Offset, SCHEMA_VERSION};
use elm_effect_journal::plan::{EffectView, plan_attempt};

pub use elm_effect_journal::plan::{DispatchPlan, ProviderCapability};

pub mod replay;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// Parsed launch configuration. Values only: no sockets, clocks, stores.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Boot {
    /// Named configuration values.
    pub config: BTreeMap<String, String>,
    /// Positional arguments.
    pub args: Vec<String>,
}

impl Boot {
    /// Parse `key=value` args (bare words land in [`Boot::args`]).
    pub fn parse(args: impl IntoIterator<Item = String>) -> Self {
        let mut boot = Self::default();
        for arg in args {
            match arg.split_once('=') {
                Some((k, v)) => {
                    boot.config.insert(k.to_string(), v.to_string());
                }
                None => boot.args.push(arg),
            }
        }
        boot
    }

    /// Read a config value.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.config.get(key).map(String::as_str)
    }
}

/// New model plus follow-up commands.
#[derive(Debug)]
pub struct Update<Model, Cmd> {
    /// Next model.
    pub model: Model,
    /// Commands to interpret next.
    pub commands: Vec<Cmd>,
}

impl<Model, Cmd> Update<Model, Cmd> {
    /// Model with no commands.
    pub fn quiet(model: Model) -> Self {
        Self {
            model,
            commands: Vec::new(),
        }
    }

    /// Model plus commands.
    pub fn with_commands(model: Model, commands: Vec<Cmd>) -> Self {
        Self { model, commands }
    }
}

/// Durability contract of one command.
#[derive(Clone, Debug, PartialEq)]
pub enum Durability {
    /// Leaves no trace.
    Ephemeral,
    /// Recorded under this key before running.
    Durable {
        /// Stable intent key.
        key: String,
    },
}

/// A command: serializable intent with a durability contract.
pub trait Cmd: Serialize + for<'de> Deserialize<'de> + Clone {
    /// This command's durability contract.
    fn durability(&self) -> Durability;

    /// What the provider offers for retry and reconciliation. The default
    /// offers nothing, so an uncertain outcome is refused rather than retried
    /// blind (ADR-0002 L14).
    fn capability(&self) -> ProviderCapability {
        ProviderCapability {
            supports_idempotency_key: false,
            supports_reconcile_query: false,
        }
    }
}

/// A pure Elm program.
pub trait Program {
    /// Application state: comparable, so loops can skip no-op renders.
    type Model: Clone + PartialEq;
    /// Incoming events (time arrives as one of these). Every message is
    /// journaled before the fold sees it, so it must serialize.
    type Msg: Send + 'static + Serialize + for<'de> Deserialize<'de>;
    /// Outgoing intents.
    type Cmd: Cmd;
    /// The typed error a command's interpreter fails with, by value.
    /// [`Failure::Fail`] carries it, and [`Interpret`] fails with it: the
    /// two change together.
    type Err: Send + 'static;

    /// Starting model plus initial commands.
    fn init(boot: &Boot) -> Update<Self::Model, Self::Cmd>;

    /// Pure fold: next model plus commands.
    fn update(model: &Self::Model, msg: &Self::Msg) -> Update<Self::Model, Self::Cmd>;

    /// How a command outcome that was not success reaches the fold.
    /// Required, so the loop never discards one: a program that cannot
    /// name this shape could only drop failures, and dropping them is
    /// the bug this method exists to prevent.
    ///
    /// [`Failure::Fail`] carries the interpreter's error type by value, so
    /// the fold matches on the error structurally rather than on its
    /// rendered text. [`Interpret`] and this method change together: the
    /// error type is the one declared on [`Program`].
    fn failure(failure: &Failure<Self::Err>) -> Self::Msg;
}

/// A command outcome that was not success. The three cause kinds stay
/// apart (ADR-0002 L2): a program that handles a typed failure
/// structurally cannot accidentally swallow a defect or an interrupt.
/// [`Failure::Fail`] hands the fold the interpreter's error by value, not
/// its `Display` text, so the fold decides on data rather than on a
/// string it has to re-parse.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Failure<E> {
    /// The interpreter's typed failure, by value. Not a rendered message:
    /// a fold that needs the text can render it itself.
    Fail(E),
    /// A defect: a panic or a broken invariant. Never a typed error value.
    Defect(#[serde(with = "DefectDef")] Defect),
    /// Interruption: not a failure, not a defect.
    Interrupted,
    /// The durable log rejected the append, so the command never ran.
    NotAppended(AppendError),
}

/// Serde mirror of the core `Defect`, which carries no serde derive. The
/// fields are public, so the mirror round-trips exactly.
#[derive(Serialize, Deserialize)]
#[serde(remote = "Defect")]
struct DefectDef {
    message: String,
    backtrace: String,
}

impl<E> Failure<E> {
    /// Classify one cause with `squash`'s priority — fail, then defect,
    /// then interrupt — so one exit yields one message. Takes the cause by
    /// value because the typed failure is handed over unrendered. First
    /// failure wins, matching Effect's `failureOption`; sibling reasons of
    /// the same kind are not preserved (the fold carries one message, like
    /// the pin). An empty cause means the runtime lost the failure: that is
    /// a bug, reported loudly as a defect rather than guessed at, and
    /// pinned by the empty-cause test.
    pub fn of_cause(cause: Cause<E>) -> Self {
        let mut defect = None;
        let mut interrupted = false;
        for reason in cause.reasons {
            match reason {
                Reason::Fail(error) => return Self::Fail(error),
                Reason::Die(d) => defect = Some(d),
                Reason::Interrupt { .. } => interrupted = true,
            }
        }
        if let Some(d) = defect {
            return Self::Defect(d);
        }
        if interrupted {
            return Self::Interrupted;
        }
        Self::Defect(Defect::new("empty cause"))
    }
}

/// Command interpreter: each command becomes an effect yielding a message.
/// The failure channel is the program's [`Program::Err`], so the error the
/// fold receives is the same type the interpreter declared.
pub trait Interpret<P: Program> {
    /// Turn one command into an effect producing the follow-up message.
    fn interpret(cmd: &P::Cmd) -> Effect<P::Msg, P::Err>;

    /// The form [`Loop`] actually calls: `interpret` with the loop's built
    /// services in hand.
    ///
    /// The two-method shape exists because `interpret` is static — it has
    /// no `self` and no parameters a host could use to hand over the world —
    /// so `use_service::<T>()` inside it was a dead end. Override this one
    /// to close that tag with [`Effect::provide_from`], and boot the loop
    /// with a `ManagedRuntime` built from a layer.
    ///
    /// The default ignores the services and calls `interpret`, so an
    /// existing interpreter keeps working unchanged and one that needs no
    /// service never names them.
    fn interpret_in_env(
        cmd: &P::Cmd,
        services: Option<&elm_effect::BuiltServices>,
    ) -> Effect<P::Msg, P::Err> {
        let _ = services;
        Self::interpret(cmd)
    }

    /// Ask the provider what happened to a command whose outcome is unknown.
    /// Used only when the command's capability offers a reconcile query. The
    /// answer is folded as the command's outcome. `None` means no query exists,
    /// so the command stays in flight.
    fn reconcile(cmd: &P::Cmd) -> Option<Effect<P::Msg, P::Err>> {
        let _ = cmd;
        None
    }
}

/// Recorded durable intent: key plus JSON body.
#[derive(Clone, Debug, PartialEq)]
pub struct Recorded {
    /// Intent key from [`Durability::Durable`].
    pub key: String,
    /// Serialized command.
    pub body: serde_json::Value,
}

/// Where a command came from: the fold step that derived it, and its position
/// in that step's command list. Outcomes and intents both carry it, so a
/// recovery can tell which command an outcome answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Origin {
    /// The fold step that derived the command. Step 0 is the init commands.
    pub step: u64,
    /// Position in that step's command list.
    pub index: usize,
}

/// A durable command that started (its intent is journaled) but has no
/// outcome in the log. Its outcome is `Unknown` (L14): the provider may have
/// run. `plan` is the dispatch decision for it, from `plan_attempt`, and
/// [`Loop::reconcile`] applies it. Recovery never re-runs it on its own.
#[derive(Clone, Debug, PartialEq)]
pub struct InFlight<C> {
    /// Which command.
    pub origin: Origin,
    /// Its intent key.
    pub key: String,
    /// The command, kept so a reconcile pass can re-dispatch or query it.
    pub cmd: C,
    /// What the planner says to do next.
    pub plan: DispatchPlan,
}

/// Read-only view of the model. There is deliberately no send path:
/// [`Loop::send`] is the one entrance.
#[derive(Clone)]
pub struct Subscriber<Model> {
    read: std::rc::Rc<dyn Fn() -> Model>,
}

impl<Model> Subscriber<Model> {
    /// Current model snapshot.
    pub fn read(&self) -> Model {
        (self.read)()
    }
}

/// Producer identity for durable appends (Law L16). `seq` is where this
/// loop resumes: a fresh host starts at 0, a restarted host reuses the
/// last committed seq so appends stay gap-free.
#[derive(Clone, Debug, PartialEq)]
pub struct Producer {
    /// Stable writer id.
    pub producer_id: String,
    /// The producer's epoch (restarts begin a new epoch).
    pub epoch: u64,
    /// Next sequence number to append.
    pub seq: u64,
}

impl Producer {
    /// A fresh producer at epoch 0, sequence 0.
    pub fn new(producer_id: impl Into<String>) -> Self {
        Self {
            producer_id: producer_id.into(),
            epoch: 0,
            seq: 0,
        }
    }
}

/// One step's still-pending commands, tagged with the step they came from.
type StepQueue<C> = (u64, VecDeque<(usize, C)>);

/// One unit of pending loop work: a message to fold, or the rest of a
/// command list still waiting for its next window.
enum Work<P: Program> {
    /// A fold input. `origin` is set for an outcome, and `None` for a host
    /// message.
    Msg { msg: P::Msg, origin: Option<Origin> },
    /// The unstarted commands of one step, with their original indices.
    Cmds {
        rest: VecDeque<(usize, P::Cmd)>,
        step: u64,
    },
}

/// The runtime loop: fold, record, interpret, feed back. The edge bridge is
/// supplied by the host (L3: interpreting a description stays at the edge,
/// not in a library crate).
///
/// # Hosting (single thread)
///
/// A `Loop` is deliberately **not `Send`**: the live model view behind
/// [`Loop::subscriber`] is an `Rc<RefCell<_>>`, so the loop and every
/// subscriber handle live on one OS thread. Host one loop per dedicated
/// thread and pump external inputs (sockets, timers, herdr events, mail
/// ingress) into [`Loop::send`] through a channel the thread drains; the
/// fold never reaches for them itself (ADR-0002 L461). Readers on other
/// threads get a snapshot over a channel, never the handle.
///
/// The [`DurableLog`] and the [`ManagedRuntime`] the loop is booted with
/// are called synchronously on this thread. A log backed by an async store
/// must answer through a request/reply channel to a runtime it does not
/// share with the caller: blocking the thread from inside a Tokio context on
/// this thread is the nested-runtime panic, so a host must not run the
/// loop on a Tokio worker thread.
///
/// A durable command whose append the log rejects (including
/// [`AppendError::StaleEpoch`] for a producer whose epoch was superseded)
/// reaches the fold as [`Failure::NotAppended`] and its provider never runs.
pub struct Loop<P: Program, I: Interpret<P>> {
    model: P::Model,
    /// Live view of the model for subscribers: updated on every fold so a
    /// handle taken earlier reads later sends, not the state at subscribe.
    view: Rc<RefCell<P::Model>>,
    /// Durable intents recorded so far, in order (cache of log appends).
    intents: Vec<Recorded>,
    /// Durable commands that started but have no outcome. See [`InFlight`].
    in_flight: Vec<InFlight<P::Cmd>>,
    /// The next fold step. Step 0 is the init commands and has no record of
    /// its own, so the first journaled message is step 1.
    steps: u64,
    /// Set when a drain stopped on a log error. Work derived before the
    /// stop is still unrun, so a snapshot now would hide it from recovery.
    halted: bool,
    rt: ManagedRuntime,
    log: Arc<dyn DurableLog>,
    producer: Producer,
    /// Commands per window (bounded map-reduce, Law L13); default 8.
    bound: usize,
    _interpret: std::marker::PhantomData<I>,
}

/// Envelope payload kinds. A journal record is a fold input (a message), a
/// durable intent, or a snapshot of the model.
const KIND_MSG: &str = "msg";
const KIND_INTENT: &str = "intent";
const KIND_SNAPSHOT: &str = "snapshot";

/// Interpretation failure.
#[derive(Debug, Clone, PartialEq, thiserror::Error, Serialize, Deserialize)]
#[error("interpretation failed: {0}")]
pub struct InterpretError(pub String);

/// Why a loop could not be rebuilt from its log.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum RecoverError {
    /// The log could not be read.
    #[error("log read failed: {0}")]
    Read(AppendError),
    /// A record is not a loop record, or its body does not decode as the
    /// program's message or model type.
    #[error("corrupt log record: {0}")]
    Corrupt(String),
    /// A pending command could not be journaled when recovery ran it.
    #[error("append failed during recovery: {0}")]
    Append(AppendError),
}

/// Why a snapshot was refused or could not be written.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum SnapshotError {
    /// The log refused the snapshot record.
    #[error("snapshot append failed: {0}")]
    Append(AppendError),
    /// Durable commands are in flight. Their outcome is unknown, so a
    /// snapshot would hide them from recovery.
    #[error("{0} durable command(s) in flight: reconcile them before a snapshot")]
    InFlight(usize),
    /// An earlier drain stopped on a log error, so unrun work exists.
    #[error("the loop halted on a log error: unrun work would be hidden by a snapshot")]
    Halted,
}

/// A durable command, as recovery sees it.
struct Derived<C> {
    step: u64,
    index: usize,
    key: String,
    cmd: C,
}

/// Durable commands of one step, tagged with their position.
fn durable_of<C: Cmd>(step: u64, commands: Vec<C>) -> Vec<Derived<C>> {
    commands
        .into_iter()
        .enumerate()
        .filter_map(|(index, cmd)| match cmd.durability() {
            Durability::Durable { key } => Some(Derived {
                step,
                index,
                key,
                cmd,
            }),
            Durability::Ephemeral => None,
        })
        .collect()
}

/// Read every record of the log, in order.
fn read_records(log: &dyn DurableLog) -> Result<Vec<serde_json::Value>, RecoverError> {
    log.read_from(&Offset::Start)
        .map_err(RecoverError::Read)?
        .into_iter()
        .map(|(_, raw)| {
            serde_json::from_str(&raw).map_err(|e| RecoverError::Corrupt(e.to_string()))
        })
        .collect()
}

/// Read an origin from a record. An absent origin is `None`; a present one
/// with a missing field is a corrupt record, never a silent `None`.
fn origin_of(value: &serde_json::Value) -> Result<Option<Origin>, RecoverError> {
    if value.is_null() {
        return Ok(None);
    }
    match (value["step"].as_u64(), value["index"].as_u64()) {
        (Some(step), Some(index)) => Ok(Some(Origin {
            step,
            index: index as usize,
        })),
        _ => Err(RecoverError::Corrupt(format!("bad origin {value}"))),
    }
}

fn indexed<C>(commands: Vec<C>) -> VecDeque<(usize, C)> {
    commands.into_iter().enumerate().collect()
}

impl<P: Program, I: Interpret<P>> Loop<P, I>
where
    P::Msg: Send + 'static,
    P::Err: Send + 'static,
{
    /// Boot the program with the host's runtime, durable log, and producer
    /// identity. The init commands run as step 0, and each durable one is
    /// journaled before it runs.
    pub fn boot(
        boot: &Boot,
        rt: ManagedRuntime,
        log: Arc<dyn DurableLog>,
        producer: Producer,
    ) -> Result<Self, AppendError> {
        let initial = P::init(boot);
        let mut this = Self::assemble(initial.model, rt, log, producer, 1);
        this.drain(vec![Work::Cmds {
            rest: indexed(initial.commands),
            step: 0,
        }])?;
        Ok(this)
    }

    /// Build a loop around a model. Nothing runs and nothing is journaled.
    fn assemble(
        model: P::Model,
        rt: ManagedRuntime,
        log: Arc<dyn DurableLog>,
        producer: Producer,
        steps: u64,
    ) -> Self {
        Self {
            model: model.clone(),
            view: Rc::new(RefCell::new(model)),
            intents: Vec::new(),
            in_flight: Vec::new(),
            steps,
            halted: false,
            rt,
            log,
            producer,
            bound: 8,
            _interpret: std::marker::PhantomData,
        }
    }

    /// Commands per window. The loop finishes one window of at most
    /// `bound` commands before the next window starts; the default is 8.
    /// `bound` is clamped to at least 1.
    pub fn with_bound(mut self, bound: usize) -> Self {
        self.bound = bound.max(1);
        self
    }

    /// The one entrance: fold one message, run resulting commands, and fold
    /// every message they produce.
    pub fn send(&mut self, msg: P::Msg) -> Result<(), AppendError> {
        self.drain(vec![Work::Msg { msg, origin: None }])
    }

    /// Current model.
    pub fn model(&self) -> &P::Model {
        &self.model
    }

    /// Recorded durable intents, in order.
    pub fn intents(&self) -> &[Recorded] {
        &self.intents
    }

    /// Durable commands that started but have no outcome. A recovery reports
    /// them and never re-runs them.
    pub fn in_flight(&self) -> &[InFlight<P::Cmd>] {
        &self.in_flight
    }

    /// Read-only subscriber handle sharing the loop's live model: a handle
    /// taken now reads later sends too.
    pub fn subscriber(&self) -> Subscriber<P::Model>
    where
        P::Model: 'static,
    {
        let view = Rc::clone(&self.view);
        Subscriber {
            read: Rc::new(move || view.borrow().clone()),
        }
    }

    fn set_model(&mut self, model: P::Model) {
        self.model = model.clone();
        *self.view.borrow_mut() = model;
    }

    /// Run pending work to completion on an explicit stack. The stack keeps
    /// the depth-first order of the old recursive loop: a window's messages
    /// are folded (with everything they cause) before the next window of the
    /// same command list starts. A stack keeps a long chain of
    /// command-to-message hops off the call stack.
    fn drain(&mut self, work: Vec<Work<P>>) -> Result<(), AppendError> {
        let result = self.run_work(work);
        if result.is_err() {
            self.halted = true;
        }
        result
    }

    fn run_work(&mut self, mut work: Vec<Work<P>>) -> Result<(), AppendError> {
        while let Some(item) = work.pop() {
            match item {
                Work::Msg { msg, origin } => {
                    // Journal before folding: a message the log refused never
                    // reaches the model, and the error stops the loop.
                    // Rejections are reported outcomes, like rejected intents:
                    // fold the failure so the model learns it, then halt.
                    // Commands the failure update returns are dropped: the
                    // journal is unavailable, so running more effects could
                    // not record their outcomes.
                    let step = self.steps;
                    if let Err(error) = self.append_message(&msg, origin) {
                        let outcomes = run_effects(
                            vec![Effect::succeed(P::failure(&Failure::NotAppended(
                                error.clone(),
                            )))],
                            &self.rt,
                        );
                        for outcome in outcomes {
                            let failure_msg = match outcome {
                                Exit::Success(msg) => msg,
                                Exit::Failure(cause) => P::failure(&Failure::of_cause(cause)),
                            };
                            let update = P::update(&self.model, &failure_msg);
                            self.set_model(update.model);
                        }
                        return Err(error);
                    }
                    let update = P::update(&self.model, &msg);
                    self.set_model(update.model);
                    if !update.commands.is_empty() {
                        work.push(Work::Cmds {
                            rest: indexed(update.commands),
                            step,
                        });
                    }
                }
                Work::Cmds { mut rest, step } => {
                    if rest.is_empty() {
                        continue;
                    }
                    let take = rest.len().min(self.bound);
                    let window: Vec<(usize, P::Cmd)> = rest.drain(..take).collect();
                    let outcomes = self.run_window(window, step);
                    if !rest.is_empty() {
                        work.push(Work::Cmds { rest, step });
                    }
                    // Reversed, so the first outcome sits on top and folds first.
                    work.extend(outcomes.into_iter().rev());
                }
            }
        }
        Ok(())
    }

    /// Append one fold input. The step counter advances only on success.
    fn append_message(&mut self, msg: &P::Msg, origin: Option<Origin>) -> Result<(), AppendError> {
        let body =
            serde_json::to_value(msg).map_err(|e| AppendError::NotAnEnvelope(e.to_string()))?;
        let origin = origin.map_or(
            serde_json::Value::Null,
            |o| serde_json::json!({ "step": o.step, "index": o.index }),
        );
        self.append_payload(
            serde_json::json!({ "kind": KIND_MSG, "body": body, "origin": origin }),
        )?;
        self.steps += 1;
        Ok(())
    }

    /// Wrap a record in the log envelope and append it. The producer stamp
    /// travels inside the record, because the log returns only the payload.
    /// The sequence number advances only on a successful append, so a retry
    /// stays gap-free.
    fn append_payload(&mut self, mut record: serde_json::Value) -> Result<Offset, AppendError> {
        let stamp = serde_json::json!({
            "producer_id": self.producer.producer_id,
            "epoch": self.producer.epoch,
            "seq": self.producer.seq,
        });
        record["stamp"] = stamp.clone();
        let envelope = serde_json::json!({
            "v": SCHEMA_VERSION,
            "producer": stamp,
            "payload": record,
        });
        let at = self.log.append_json(&envelope.to_string())?;
        self.producer.seq += 1;
        Ok(at)
    }

    /// Map one window to its outcomes. The durable intent is journaled first,
    /// in order, and a rejected intent reports `NotAppended` with its provider
    /// never run. Every outcome carries its origin, so a recovery can tell
    /// which command it answers.
    fn run_window(&mut self, window: Vec<(usize, P::Cmd)>, step: u64) -> Vec<Work<P>> {
        let origins: Vec<Origin> = window
            .iter()
            .map(|(index, _)| Origin {
                step,
                index: *index,
            })
            .collect();
        let mut effects: Vec<Effect<P::Msg, P::Err>> = Vec::with_capacity(window.len());
        for (index, cmd) in &window {
            if let Durability::Durable { key } = cmd.durability() {
                let origin = Origin {
                    step,
                    index: *index,
                };
                match self.append_durable(&key, cmd, origin, 1) {
                    Ok(recorded) => self.intents.push(recorded),
                    Err(error) => {
                        effects.push(Effect::succeed(P::failure(&Failure::NotAppended(error))));
                        continue;
                    }
                }
            }
            effects.push(I::interpret_in_env(cmd, self.rt.context()));
        }
        run_effects(effects, &self.rt)
            .into_iter()
            .zip(origins)
            .map(|(outcome, origin)| {
                let msg = match outcome {
                    Exit::Success(msg) => msg,
                    Exit::Failure(cause) => P::failure(&Failure::of_cause(cause)),
                };
                Work::Msg {
                    msg,
                    origin: Some(origin),
                }
            })
            .collect()
    }

    /// Append one durable intent. The payload names its origin, so recovery
    /// can match it to the command it started.
    fn append_durable(
        &mut self,
        key: &str,
        cmd: &P::Cmd,
        origin: Origin,
        attempt: i64,
    ) -> Result<Recorded, AppendError> {
        let body =
            serde_json::to_value(cmd).map_err(|e| AppendError::NotAnEnvelope(e.to_string()))?;
        self.append_payload(serde_json::json!({
            "kind": KIND_INTENT,
            "step": origin.step,
            "index": origin.index,
            "attempt": attempt,
            "key": key,
            "body": body.clone(),
        }))?;
        Ok(Recorded {
            key: key.to_string(),
            body,
        })
    }

    /// Apply the dispatch plan to every in-flight command. Each one is
    /// `Unknown`, so the planner decides:
    ///
    /// - `ReconcileFirst`: ask the provider with [`Interpret::reconcile`]. The
    ///   answer is folded as that command's outcome.
    /// - `RetrySameKey`: re-dispatch under the same key as the next attempt.
    /// - anything else: the command stays in flight and is still reported.
    ///
    /// Returns how many commands got an outcome. It is explicit, not automatic,
    /// because a reconcile may call a provider: the host decides when to run it.
    /// A failed append stops the pass and leaves the rest in flight.
    pub fn reconcile(&mut self) -> Result<usize, AppendError> {
        let mut resolved = 0;
        let mut kept = Vec::new();
        let mut entries = std::mem::take(&mut self.in_flight).into_iter();
        while let Some(entry) = entries.next() {
            match self.reconcile_one(&entry) {
                Ok(Some(work)) => match self.drain(vec![work]) {
                    Ok(()) => resolved += 1,
                    Err(error) => {
                        kept.push(entry);
                        kept.extend(entries);
                        self.in_flight = kept;
                        return Err(error);
                    }
                },
                Ok(None) => kept.push(entry),
                Err(error) => {
                    kept.push(entry);
                    kept.extend(entries);
                    self.in_flight = kept;
                    return Err(error);
                }
            }
        }
        self.in_flight = kept;
        Ok(resolved)
    }

    /// Run one in-flight command's plan. `Ok(None)` means it stays in flight.
    fn reconcile_one(&mut self, entry: &InFlight<P::Cmd>) -> Result<Option<Work<P>>, AppendError> {
        let origin = entry.origin;
        let effect = match &entry.plan {
            DispatchPlan::ReconcileFirst { .. } => match I::reconcile(&entry.cmd) {
                Some(effect) => effect,
                None => return Ok(None),
            },
            DispatchPlan::RetrySameKey { attempt } => {
                let recorded = self.append_durable(&entry.key, &entry.cmd, origin, *attempt)?;
                self.intents.push(recorded);
                I::interpret_in_env(&entry.cmd, self.rt.context())
            }
            _ => return Ok(None),
        };
        let msg = match run_effects(vec![effect], &self.rt).pop() {
            Some(Exit::Success(msg)) => msg,
            Some(Exit::Failure(cause)) => P::failure(&Failure::of_cause(cause)),
            None => return Ok(None),
        };
        Ok(Some(Work::Msg {
            msg,
            origin: Some(origin),
        }))
    }
}

impl<P: Program, I: Interpret<P>> Loop<P, I>
where
    P::Msg: Send + 'static,
    P::Err: Send + 'static,
    P::Model: Serialize + DeserializeOwned,
{
    /// Rebuild a loop from its durable log.
    ///
    /// The last snapshot is the starting model, so only the records after it
    /// are folded again. Without a snapshot, recovery folds from step 0. No
    /// provider runs during the fold. Then the durable commands that never
    /// started run once, and each one is journaled as it starts. A command
    /// whose intent exists but whose outcome does not is in flight. It is
    /// reported in [`Loop::in_flight`] and never re-run. The producer resumes
    /// after its own last record.
    ///
    /// Ephemeral commands leave no trace, so an ephemeral command that was
    /// derived but not finished is not replayed.
    pub fn recover(
        boot: &Boot,
        rt: ManagedRuntime,
        log: Arc<dyn DurableLog>,
        mut producer: Producer,
    ) -> Result<Self, RecoverError> {
        let records = read_records(log.as_ref())?;

        let mut last_seq = None;
        for record in &records {
            let stamp = &record["stamp"];
            if stamp["producer_id"].as_str() == Some(producer.producer_id.as_str())
                && stamp["epoch"].as_u64() == Some(producer.epoch)
            {
                last_seq = stamp["seq"].as_u64();
            }
        }
        producer.seq = last_seq.map_or(0, |seq| seq + 1);

        let initial = P::init(boot);
        let snapshot_at = records
            .iter()
            .rposition(|r| r["kind"].as_str() == Some(KIND_SNAPSHOT));
        let mut steps: u64 = 1;
        let (mut model, from) = match snapshot_at {
            Some(at) => {
                let model = serde_json::from_value(records[at]["model"].clone())
                    .map_err(|e| RecoverError::Corrupt(e.to_string()))?;
                steps = records[at]["steps"].as_u64().ok_or_else(|| {
                    RecoverError::Corrupt("snapshot without a step counter".into())
                })?;
                (model, at + 1)
            }
            None => (initial.model, 0),
        };

        // Step 0 is the init commands. A snapshot already covers them.
        let mut derived = if from == 0 {
            durable_of(0, initial.commands)
        } else {
            Vec::new()
        };
        let mut resolved: HashSet<(u64, usize)> = HashSet::new();
        let mut started: HashSet<(u64, usize)> = HashSet::new();
        // The highest attempt journaled for each command. The planner needs it.
        let mut attempts: HashMap<(u64, usize), i64> = HashMap::new();
        let mut intents = Vec::new();

        for (at, record) in records.iter().enumerate() {
            match record["kind"].as_str() {
                Some(KIND_MSG) => {
                    let step = steps;
                    steps += 1;
                    if let Some(origin) = origin_of(&record["origin"])? {
                        resolved.insert((origin.step, origin.index));
                    }
                    if at < from {
                        continue;
                    }
                    let msg: P::Msg = serde_json::from_value(record["body"].clone())
                        .map_err(|e| RecoverError::Corrupt(e.to_string()))?;
                    let update = P::update(&model, &msg);
                    model = update.model;
                    derived.extend(durable_of(step, update.commands));
                }
                Some(KIND_INTENT) => {
                    let (Some(step), Some(index)) =
                        (record["step"].as_u64(), record["index"].as_u64())
                    else {
                        return Err(RecoverError::Corrupt(format!(
                            "intent without origin: {record}"
                        )));
                    };
                    let id = (step, index as usize);
                    let attempt = record["attempt"].as_i64().unwrap_or(1);
                    attempts
                        .entry(id)
                        .and_modify(|a| *a = (*a).max(attempt))
                        .or_insert(attempt);
                    started.insert(id);
                    intents.push(Recorded {
                        key: record["key"].as_str().unwrap_or_default().to_string(),
                        body: record["body"].clone(),
                    });
                }
                Some(KIND_SNAPSHOT) => {}
                other => {
                    return Err(RecoverError::Corrupt(format!(
                        "unknown record kind {other:?}"
                    )));
                }
            }
        }

        let mut pending: Vec<Derived<P::Cmd>> = Vec::new();
        let mut in_flight = Vec::new();
        for d in derived {
            let id = (d.step, d.index);
            if resolved.contains(&id) {
                continue;
            }
            if started.contains(&id) {
                let attempt = attempts.get(&id).copied().unwrap_or(1);
                let plan = plan_attempt(
                    Some(&EffectView {
                        state: EffectState::Unknown,
                        attempt,
                        lease_live: false,
                    }),
                    &d.cmd.capability(),
                );
                in_flight.push(InFlight {
                    origin: Origin {
                        step: d.step,
                        index: d.index,
                    },
                    key: d.key,
                    cmd: d.cmd,
                    plan,
                });
            } else {
                pending.push(d);
            }
        }

        // One work item per step, so each step's pending commands keep their
        // order and their origin. Reversed: the first step sits on top.
        let mut groups: Vec<StepQueue<P::Cmd>> = Vec::new();
        for d in pending {
            match groups.last_mut() {
                Some((step, rest)) if *step == d.step => rest.push_back((d.index, d.cmd)),
                _ => groups.push((d.step, VecDeque::from([(d.index, d.cmd)]))),
            }
        }
        let work: Vec<Work<P>> = groups
            .into_iter()
            .rev()
            .map(|(step, rest)| Work::Cmds { rest, step })
            .collect();

        let mut this = Self::assemble(model, rt, log, producer, steps);
        this.intents = intents;
        this.in_flight = in_flight;
        this.drain(work).map_err(RecoverError::Append)?;
        Ok(this)
    }

    /// Write a snapshot of the model, then drop the log records it covers.
    /// The log then holds the snapshot and what came after it, so recovery
    /// and the log both stay bounded. Refused while durable commands are in
    /// flight, or after a drain halted on a log error, because either would
    /// hide unrun work from recovery.
    ///
    /// After a recovery, [`Loop::intents`] covers only records after the last
    /// snapshot. Earlier intents were compacted away.
    pub fn snapshot(&mut self) -> Result<(), SnapshotError> {
        if !self.in_flight.is_empty() {
            return Err(SnapshotError::InFlight(self.in_flight.len()));
        }
        if self.halted {
            return Err(SnapshotError::Halted);
        }
        let model = serde_json::to_value(&self.model)
            .map_err(|e| SnapshotError::Append(AppendError::NotAnEnvelope(e.to_string())))?;
        // The step counter travels with the snapshot. Compaction drops the
        // records that would otherwise let recovery count steps again.
        let at = self
            .append_payload(
                serde_json::json!({ "kind": KIND_SNAPSHOT, "model": model, "steps": self.steps }),
            )
            .map_err(SnapshotError::Append)?;
        self.log
            .compact_before(&at)
            .map_err(SnapshotError::Append)?;
        Ok(())
    }
}

/// Run one window's effects, concurrently. The runtime's edge already turns
/// a panic into a defect, and [`ManagedRuntime`] is `Sync`, so commands
/// in a window may overlap. Results keep their position, so the caller sees
/// them in command order no matter how the threads interleaved.
///
/// A window of one runs inline: there is nothing to overlap, and a thread
/// would cost more than the work.
///
/// The loop does not catch panics itself (ADR-0002 L2, supervisor only). A
/// thread that dies outside the runtime's guard makes `join` fail, and that
/// becomes a defect for its command alone, so the fold keeps every other
/// outcome.
fn run_effects<A, E>(effects: Vec<Effect<A, E>>, rt: &ManagedRuntime) -> Vec<Exit<A, E>>
where
    A: Send + 'static,
    E: Send + 'static,
{
    if effects.len() < 2 {
        return effects.into_iter().map(|effect| rt.run(effect)).collect();
    }
    std::thread::scope(|scope| {
        let handles: Vec<_> = effects
            .into_iter()
            .map(|effect| scope.spawn(|| rt.run(effect)))
            .collect();
        handles
            .into_iter()
            .map(|handle| match handle.join() {
                Ok(exit) => exit,
                Err(_) => Exit::Failure(Cause::die(Defect::new(
                    "command thread died before returning an outcome",
                ))),
            })
            .collect()
    })
}
