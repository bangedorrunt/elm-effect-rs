// ADR-0008: Zero-cost microstep observer hook (Law L10)
// See: docs/decisions/0008-harden-elm-effect-xstate-with-compiled-topology-timer-tokens-and-macro-dx.md

use crate::configuration::Configuration;
use crate::types::{StepOutcome, TimerIntent};

/// Combined result returned by observed single-step execution (`microsteps`).
pub type MicrostepTrace<S, C, CMD, E> =
    (StepOutcome<S, C, CMD, E>, Vec<MicrostepRecord<S, C, CMD>>);

/// Which phase of a macrostep produced a [`MicrostepRecord`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum MicrostepKind {
    /// Initial statechart entry (`Machine::enter` / `Machine::start`).
    Init,
    /// Selection and transition for an external event.
    Event,
    /// Selection and transition for an elapsed `after` timer.
    Timer,
    /// Eventless (`always`) microstep cascading toward quiescence.
    Always,
    /// Compound or parallel completion (`on_done`) microstep.
    Done,
    /// Internal event raised by `enq.raise`.
    Raised,
}

/// Captured trace of a single microstep during a statechart step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MicrostepRecord<S, C, CMD> {
    /// Why this microstep ran.
    pub kind: MicrostepKind,
    /// States exited in deepest-first order.
    pub exited: Vec<S>,
    /// States entered in ancestor-to-descendant order.
    pub entered: Vec<S>,
    /// Active leaf configuration after the microstep.
    pub configuration: Configuration<S>,
    /// Extended context snapshot after the microstep.
    pub context: C,
    /// Commands emitted during this microstep.
    pub commands: Vec<CMD>,
    /// Timer intents emitted during this microstep.
    pub timers: Vec<TimerIntent<S>>,
}

/// Hook notified after each microstep when observation is enabled.
///
/// Implementations that set `ENABLED = false` (such as `()`) allow the compiler
/// to eliminate all observation overhead on the hot path.
pub trait Observer<S, C, CMD> {
    /// Compile-time toggle; when `false`, no microstep records are built.
    const ENABLED: bool = true;

    /// Receive one microstep record in exact execution order.
    fn on_microstep(&mut self, record: MicrostepRecord<S, C, CMD>);
}

impl<S, C, CMD> Observer<S, C, CMD> for () {
    const ENABLED: bool = false;

    #[inline]
    fn on_microstep(&mut self, _record: MicrostepRecord<S, C, CMD>) {}
}

impl<S, C, CMD> Observer<S, C, CMD> for Vec<MicrostepRecord<S, C, CMD>> {
    fn on_microstep(&mut self, record: MicrostepRecord<S, C, CMD>) {
        self.push(record);
    }
}
