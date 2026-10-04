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
//! no send path by construction (see `subscriber_cannot_write`).

use std::collections::BTreeMap;

use elm_effect::{Effect, Exit, Runtime};
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

/// The runtime loop: fold, record, interpret, feed back.
pub struct Loop<P: Program, I: Interpret<P>> {
    model: P::Model,
    intents: Vec<Recorded>,
    _interpret: std::marker::PhantomData<I>,
}

impl<P: Program, I: Interpret<P>> Loop<P, I>
where
    P::Msg: Send + 'static,
    I::Error: Send + 'static,
{
    /// Boot the program: run `init`, record and run initial commands.
    pub fn boot(boot: &Boot) -> Self {
        let mut this = Self {
            model: P::init(boot).model,
            intents: Vec::new(),
            _interpret: std::marker::PhantomData,
        };
        let initial = P::init(boot);
        this.model = initial.model;
        let commands = initial.commands;
        this.run_commands(commands);
        this
    }

    /// The one entrance: fold one message, run resulting commands.
    pub fn send(&mut self, msg: P::Msg) {
        let update = P::update(&self.model, &msg);
        self.model = update.model;
        self.run_commands(update.commands);
    }

    /// Current model.
    pub fn model(&self) -> &P::Model {
        &self.model
    }

    /// Recorded durable intents, in order.
    pub fn intents(&self) -> &[Recorded] {
        &self.intents
    }

    /// Read-only subscriber handle.
    pub fn subscriber(&self) -> Subscriber<P::Model>
    where
        P::Model: 'static,
    {
        let model = self.model.clone();
        Subscriber { read: std::rc::Rc::new(move || model.clone()) }
    }

    fn run_commands(&mut self, commands: Vec<P::Cmd>) {
        let mut queue: Vec<P::Msg> = Vec::new();
        for cmd in &commands {
            if let Durability::Durable { key } = cmd.durability() {
                if let Ok(body) = serde_json::to_value(cmd) {
                    self.intents.push(Recorded { key, body });
                }
            }
        }
        for cmd in commands {
            match Runtime::run(I::interpret(&cmd)) {
                Exit::Success(msg) => queue.push(msg),
                Exit::Failure(_) => {}
            }
        }
        for msg in queue {
            self.send(msg);
        }
    }
}

/// Interpretation failure.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("interpretation failed: {0}")]
pub struct InterpretError(pub String);
