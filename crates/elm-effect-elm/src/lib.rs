//! Elm loop on Effect (ADR-0002 Phase 8, Law L12 structure).
//!
//! A [`Program`] is pure: `init` from parsed [`Boot`] config, `update`
//! folding `(Model, Msg)` into [`Update`] (new model plus commands).
//! Time enters as a message; the host samples the clock and sends `Msg`.
//! No live socket, clock, or store lives in here. [`Interpret`] turns
//! each command into an [`Effect`](elm_effect::Effect); the [`Loop`] runs
//! it on a [`Runtime`](elm_effect::Runtime) and feeds the outcome back as
//! a message. One entrance ([`Loop::send`]), one exit (the model).
//! Durable commands record their JSON intent under their key before
//! running; [`Ephemeral`](Durability::Ephemeral) commands leave no trace.
//! [`Subscriber`] reads the model and cannot write it: the handle exposes
//! no send path by construction (see `subscriber_cannot_write`). No
//! command outcome is lost: success keeps the interpreter's message, and
//! every failure, defect, and interrupt enters through
//! [`Program::failure`] as a [`Failure`].

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::Arc;

use elm_effect::{Cause, Effect, Exit, ManagedRuntime};
use elm_effect_journal::durable_log::{AppendError, DurableLog, SCHEMA_VERSION};

pub mod replay;
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
        Self { model, commands: Vec::new() }
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
}

/// A pure Elm program.
pub trait Program {
    /// Application state: comparable, so loops can skip no-op renders.
    type Model: Clone + PartialEq;
    /// Incoming events (time arrives as one of these).
    type Msg: Send + 'static;
    /// Outgoing intents.
    type Cmd: Cmd;

    /// Starting model plus initial commands.
    fn init(boot: &Boot) -> Update<Self::Model, Self::Cmd>;

    /// Pure fold: next model plus commands.
    fn update(model: &Self::Model, msg: &Self::Msg) -> Update<Self::Model, Self::Cmd>;

    /// How a command outcome that was not success reaches the fold.
    /// Required, so the loop never discards one: a program that cannot
    /// name this shape could only drop failures, and dropping them is
    /// the bug this method exists to prevent.
    fn failure(failure: &Failure) -> Self::Msg;
}

/// A command outcome that was not success. The three cause kinds stay
/// apart (ADR-0002 L2): a program that handles a typed failure
/// structurally cannot accidentally swallow a defect or an interrupt.
/// The typed value arrives here rendered by its `Display`, because
/// [`Program`] cannot name the interpreter's error type; the kind does
/// not collapse.
#[derive(Clone, Debug, PartialEq)]
pub enum Failure {
    /// The interpreter's typed failure, rendered.
    Fail(String),
    /// A defect: a panic or a broken invariant. Never a typed error value.
    Defect(String),
    /// Interruption: not a failure, not a defect.
    Interrupted,
    /// The durable log rejected the append, so the command never ran.
    NotAppended(String),
}

impl Failure {
    /// Classify one cause with `squash`'s priority — fail, then defect,
    /// then interrupt — so one exit yields one message. A cause carrying
    /// several kinds reports the first, in that order.
    fn of_cause<E: std::fmt::Display>(cause: &Cause<E>) -> Self {
        if let Some(error) = cause.failure() {
            return Self::Fail(error.to_string());
        }
        if let Some(defect) = cause.defects().first() {
            return Self::Defect(defect.message.clone());
        }
        if cause.is_interrupted() {
            return Self::Interrupted;
        }
        Self::Defect("empty cause".to_string())
    }
}

/// Command interpreter: each command becomes an effect yielding a message.
pub trait Interpret<P: Program> {
    /// Failure shape of interpretation.
    type Error: std::fmt::Display + Send + 'static;

    /// Turn one command into an effect producing the follow-up message.
    fn interpret(cmd: &P::Cmd) -> Effect<P::Msg, Self::Error>;
}

/// Recorded durable intent: key plus JSON body.
#[derive(Clone, Debug, PartialEq)]
pub struct Recorded {
    /// Intent key from [`Durability::Durable`].
    pub key: String,
    /// Serialized command.
    pub body: serde_json::Value,
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
        Self { producer_id: producer_id.into(), epoch: 0, seq: 0 }
    }
}

/// The runtime loop: fold, record, interpret, feed back. The edge bridge is
/// supplied by the host (L3: interpreting a description stays at the edge,
/// not in a library crate).
pub struct Loop<P: Program, I: Interpret<P>> {
    model: P::Model,
    /// Live view of the model for subscribers: updated on every fold so a
    /// handle taken earlier reads later sends, not the state at subscribe.
    view: Rc<RefCell<P::Model>>,
    /// Durable intents recorded so far, in order (cache of log appends).
    intents: Vec<Recorded>,
    rt: ManagedRuntime,
    log: Arc<dyn DurableLog>,
    producer: Producer,
    /// Commands per window (bounded map-reduce, Law L13); default 8.
    bound: usize,
    _interpret: std::marker::PhantomData<I>,
}

impl<P: Program, I: Interpret<P>> Loop<P, I>
where
    P::Msg: Send + 'static,
    I::Error: Send + 'static,
{
    /// Boot the program with the host's runtime, durable log, and producer
    /// identity. Durable commands append to `log` before they run.
    pub fn boot(
        boot: &Boot,
        rt: ManagedRuntime,
        log: Arc<dyn DurableLog>,
        producer: Producer,
    ) -> Self {
        let initial = P::init(boot);
        let mut this = Self {
            model: initial.model.clone(),
            view: Rc::new(RefCell::new(initial.model)),
            intents: Vec::new(),
            rt,
            log,
            producer,
            bound: 8,
            _interpret: std::marker::PhantomData,
        };
        this.run_commands(initial.commands);
        this
    }

    /// Commands per window. `run_commands` finishes one window of at most
    /// `bound` commands before the next window starts; the default is 8.
    /// `bound` is clamped to at least 1.
    pub fn with_bound(mut self, bound: usize) -> Self {
        self.bound = bound.max(1);
        self
    }

    /// The one entrance: fold one message, run resulting commands.
    pub fn send(&mut self, msg: P::Msg) {
        let update = P::update(&self.model, &msg);
        self.set_model(update.model);
        self.run_commands(update.commands);
    }

    fn set_model(&mut self, model: P::Model) {
        self.model = model.clone();
        *self.view.borrow_mut() = model;
    }

    /// Current model.
    pub fn model(&self) -> &P::Model {
        &self.model
    }

    /// Recorded durable intents, in order.
    pub fn intents(&self) -> &[Recorded] {
        &self.intents
    }

    /// Read-only subscriber handle sharing the loop's live model: a handle
    /// taken now reads later sends too.
    pub fn subscriber(&self) -> Subscriber<P::Model>
    where
        P::Model: 'static,
    {
        let view = Rc::clone(&self.view);
        Subscriber { read: Rc::new(move || view.borrow().clone()) }
    }

    /// Bounded map-reduce (Law L13): split the command list into windows of
    /// at most `bound`; map each command (append the durable intent first,
    /// then run it synchronously) and reduce the outcomes through the fold
    /// before the next window starts. Nothing is dropped: success keeps the
    /// interpreter's message, a failure classifies into a [`Failure`], and
    /// a rejected append reports one instead of skipping the command in
    /// silence. Message order equals command order.
    fn run_commands(&mut self, commands: Vec<P::Cmd>) {
        for window in commands.chunks(self.bound) {
            let mut msgs = Vec::with_capacity(window.len());
            for cmd in window {
                msgs.push(self.run_one(cmd));
            }
            for msg in msgs {
                self.send(msg);
            }
        }
    }

    /// Map one command to its message. A durable intent appends through the
    /// log before the provider runs; a rejected append is reported to the
    /// fold and the command is not run, so no provider outruns its record.
    fn run_one(&mut self, cmd: &P::Cmd) -> P::Msg {
        if let Durability::Durable { key } = cmd.durability() {
            match self.append_durable(&key, cmd) {
                Ok(recorded) => self.intents.push(recorded),
                Err(error) => {
                    return P::failure(&Failure::NotAppended(format!(
                        "durable append rejected: {error}"
                    )));
                }
            }
        }
        match self.rt.run(I::interpret(cmd)) {
            Exit::Success(msg) => msg,
            Exit::Failure(cause) => P::failure(&Failure::of_cause(&cause)),
        }
    }

    /// Append one durable intent through the log. The payload is the same
    /// `{key, body}` pair `intents()` reports. Consumes a sequence number
    /// only on a successful append, so a retry stays gap-free.
    fn append_durable(&mut self, key: &str, cmd: &P::Cmd) -> Result<Recorded, AppendError> {
        let body = serde_json::to_value(cmd)
            .map_err(|e| AppendError::NotAnEnvelope(e.to_string()))?;
        let envelope = serde_json::json!({
            "v": SCHEMA_VERSION,
            "producer": {
                "producer_id": self.producer.producer_id,
                "epoch": self.producer.epoch,
                "seq": self.producer.seq,
            },
            "payload": { "key": key, "body": body },
        });
        self.log.append_json(&envelope.to_string())?;
        self.producer.seq += 1;
        Ok(Recorded { key: key.to_string(), body })
    }
}

/// Interpretation failure.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("interpretation failed: {0}")]
pub struct InterpretError(pub String);
