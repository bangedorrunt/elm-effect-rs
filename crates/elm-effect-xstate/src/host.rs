// ADR-0008: Generation-token host timer bookkeeping & clock-aware persistence (Law L9)
// See: docs/decisions/0008-harden-elm-effect-xstate-with-compiled-topology-timer-tokens-and-macro-dx.md

use crate::machine::Machine;
use crate::types::{MachineState, StepOutcome, TimerFired, TimerIntent};
use elm_effect::Duration;

/// Monotonic token distinguishing one timer arm from earlier or later arms of the
/// same state/duration so stale queued timer callbacks cannot fire early.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
pub struct TimerToken(pub u64);

/// Operation the host timer scheduler must perform after a machine step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TimerOp<S, H> {
    /// Start a host timer due after `delay` and register its handle via
    /// [`HostTimers::armed`] with `token`.
    Arm {
        /// State that declared the `after` transition.
        state: S,
        /// Delay until the timer expires.
        duration: Duration,
        /// Unique generation token for this arm.
        token: TimerToken,
    },
    /// Stop an active host timer handle because its state exited or re-armed.
    Disarm(H),
}

/// Clock-anchored timer record persisted across restarts (ADR-0008 Law L9 / D19).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(bound(
    serialize = "S: serde::Serialize",
    deserialize = "S: serde::Deserialize<'de>"
))]
pub struct PersistedTimer<S> {
    /// State that armed the timer.
    pub state: S,
    /// Original declared delay duration on the machine.
    #[serde(with = "crate::types::duration_string")]
    pub duration: Duration,
    /// Host clock timestamp in milliseconds when the timer was originally armed.
    pub scheduled_at_ms: u64,
}

#[derive(Debug, Clone)]
struct PendingTimer<S, H> {
    state: S,
    duration: Duration,
    token: TimerToken,
    handle: Option<H>,
    scheduled_at_ms: Option<u64>,
}

/// Host-side timer coordinator mapping [`TimerIntent`]s into generation-fenced
/// [`TimerOp`]s and rejecting stale timer wakeups.
#[derive(Debug)]
pub struct HostTimers<S, H> {
    pending: Vec<PendingTimer<S, H>>,
    next_token: u64,
}

impl<S, H> Default for HostTimers<S, H> {
    fn default() -> Self {
        Self::new()
    }
}

impl<S, H> HostTimers<S, H> {
    /// Create an empty timer table.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            pending: Vec::new(),
            next_token: 1,
        }
    }

    /// Number of currently pending timers.
    #[must_use]
    pub fn len(&self) -> usize {
        self.pending.len()
    }

    /// Whether no timers are pending.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    /// Attach the host's timer `handle` to the pending arm matching `token`.
    ///
    /// Returns `Some(handle)` back if `token` has already been cancelled or
    /// superseded, or if replacing an older handle on the same slot.
    #[must_use = "a returned handle belongs to a stale or superseded timer and should be cancelled"]
    pub fn armed(&mut self, token: TimerToken, handle: H) -> Option<H> {
        match self.pending.iter_mut().find(|entry| entry.token == token) {
            Some(entry) => entry.handle.replace(handle),
            None => Some(handle),
        }
    }

    /// Consume a fired timer `token`, returning `None` if the token is stale.
    pub fn fired(&mut self, token: TimerToken) -> Option<TimerFired<S>> {
        let idx = self.pending.iter().position(|entry| entry.token == token)?;
        let entry = self.pending.swap_remove(idx);
        Some(TimerFired::new(entry.state, entry.duration))
    }

    /// Iterate over all pending `(state, duration, token, handle)` entries.
    pub fn iter(&self) -> impl Iterator<Item = (&S, Duration, TimerToken, Option<&H>)> {
        self.pending.iter().map(|entry| {
            (
                &entry.state,
                entry.duration,
                entry.token,
                entry.handle.as_ref(),
            )
        })
    }
}

impl<S: Clone + PartialEq, H> HostTimers<S, H> {
    /// Convert a slice of [`TimerIntent`]s into netted [`TimerOp`]s.
    pub fn apply(&mut self, intents: &[TimerIntent<S>], ops: &mut Vec<TimerOp<S, H>>) {
        self.apply_inner(intents, None, ops);
    }

    /// Convert [`TimerIntent`]s into [`TimerOp`]s while stamping new timers with `now_ms`.
    pub fn apply_at(
        &mut self,
        intents: &[TimerIntent<S>],
        now_ms: u64,
        ops: &mut Vec<TimerOp<S, H>>,
    ) {
        self.apply_inner(intents, Some(now_ms), ops);
    }

    fn apply_inner(
        &mut self,
        intents: &[TimerIntent<S>],
        now_ms: Option<u64>,
        ops: &mut Vec<TimerOp<S, H>>,
    ) {
        let start = ops.len();
        for intent in intents {
            match intent {
                TimerIntent::Start { state, duration } => {
                    self.cancel_matching(state, *duration, ops, start);
                    self.arm_entry(state.clone(), *duration, *duration, now_ms, ops);
                }
                TimerIntent::Cancel { state, duration } => {
                    self.cancel_matching(state, *duration, ops, start);
                }
            }
        }
    }

    /// Whether a timer matching `(state, duration)` is currently pending.
    #[must_use]
    pub fn contains(&self, state: &S, duration: Duration) -> bool {
        self.pending
            .iter()
            .any(|entry| entry.state == *state && entry.duration == duration)
    }

    /// The host handle currently bound to `(state, duration)`, if any.
    #[must_use]
    pub fn handle(&self, state: &S, duration: Duration) -> Option<&H> {
        self.pending
            .iter()
            .find(|entry| entry.state == *state && entry.duration == duration)
            .and_then(|entry| entry.handle.as_ref())
    }

    /// Snapshot pending timers with their arming timestamp for clock-aware restore.
    #[must_use]
    pub fn persist(&self) -> Vec<PersistedTimer<S>> {
        self.pending
            .iter()
            .map(|entry| PersistedTimer {
                state: entry.state.clone(),
                duration: entry.duration,
                scheduled_at_ms: entry.scheduled_at_ms.unwrap_or(0),
            })
            .collect()
    }

    /// Disarm all currently pending timers and re-arm `persisted` timers using their
    /// remaining delay `(scheduled_at_ms + duration).saturating_sub(now_ms)`.
    pub fn restore_remaining(
        &mut self,
        persisted: &[PersistedTimer<S>],
        now_ms: u64,
        ops: &mut Vec<TimerOp<S, H>>,
    ) {
        ops.extend(
            self.pending
                .drain(..)
                .filter_map(|entry| entry.handle.map(TimerOp::Disarm)),
        );
        for entry in persisted {
            let deadline_ms = entry
                .scheduled_at_ms
                .saturating_add(entry.duration.as_millis());
            let remaining = Duration::from_millis(deadline_ms.saturating_sub(now_ms));
            self.arm_entry(
                entry.state.clone(),
                entry.duration,
                remaining,
                Some(entry.scheduled_at_ms),
                ops,
            );
        }
    }

    fn arm_entry(
        &mut self,
        state: S,
        declared_duration: Duration,
        schedule_delay: Duration,
        scheduled_at_ms: Option<u64>,
        ops: &mut Vec<TimerOp<S, H>>,
    ) {
        let token = TimerToken(self.next_token);
        self.next_token += 1;
        self.pending.push(PendingTimer {
            state: state.clone(),
            duration: declared_duration,
            token,
            handle: None,
            scheduled_at_ms,
        });
        ops.push(TimerOp::Arm {
            state,
            duration: schedule_delay,
            token,
        });
    }

    fn cancel_matching(
        &mut self,
        state: &S,
        duration: Duration,
        ops: &mut Vec<TimerOp<S, H>>,
        start: usize,
    ) {
        if let Some(op_pos) = ops[start..]
            .iter()
            .position(|op| matches!(op, TimerOp::Arm { state: s, duration: d, .. } if s == state && *d == duration))
        {
            if let TimerOp::Arm { token, .. } = ops.remove(start + op_pos)
                && let Some(pos) = self.pending.iter().position(|entry| entry.token == token)
            {
                self.pending.swap_remove(pos);
            }
            return;
        }
        if let Some(pos) = self
            .pending
            .iter()
            .position(|entry| entry.state == *state && entry.duration == duration)
            && let Some(handle) = self.pending.swap_remove(pos).handle
        {
            ops.push(TimerOp::Disarm(handle));
        }
    }
}

/// Host scheduler callback interface used by [`DrivenMachine`].
pub trait HostScheduler<S, H> {
    /// Arm a timer that triggers after `delay` and hands `token` back to [`DrivenMachine::fire`].
    fn schedule(&mut self, state: &S, delay: Duration, token: TimerToken) -> H;

    /// Cancel a previously scheduled timer handle.
    fn cancel(&mut self, handle: H);
}

impl<S, H, T: HostScheduler<S, H> + ?Sized> HostScheduler<S, H> for &mut T {
    fn schedule(&mut self, state: &S, delay: Duration, token: TimerToken) -> H {
        (**self).schedule(state, delay, token)
    }

    fn cancel(&mut self, handle: H) {
        (**self).cancel(handle);
    }
}

/// Stateful host runner pairing a [`Machine`] with its [`MachineState`] and
/// generation-fenced [`HostTimers`].
pub struct DrivenMachine<'a, S: Ord + Clone, E, C: Clone, CMD, H> {
    machine: &'a Machine<S, E, C, CMD>,
    state: MachineState<S, C>,
    timers: HostTimers<S, H>,
    ops: Vec<TimerOp<S, H>>,
}

impl<'a, S, E, C, CMD, H> DrivenMachine<'a, S, E, C, CMD, H>
where
    S: Ord + Clone + std::fmt::Debug + Send + Sync + 'static,
    E: Clone + Send + Sync + 'static,
    C: Clone + Send + Sync + 'static,
    CMD: Clone + Send + Sync + 'static,
{
    /// Enter and settle a machine, scheduling its initial timers on `scheduler`.
    pub fn start(
        machine: &'a Machine<S, E, C, CMD>,
        context: &C,
        init: &E,
        scheduler: impl HostScheduler<S, H>,
    ) -> (Self, StepOutcome<S, C, CMD, E>) {
        Self::start_inner(machine, context, init, None, scheduler)
    }

    /// Same as [`start`](Self::start), stamping armed timers with `now_ms`.
    pub fn start_at(
        machine: &'a Machine<S, E, C, CMD>,
        context: &C,
        init: &E,
        now_ms: u64,
        scheduler: impl HostScheduler<S, H>,
    ) -> (Self, StepOutcome<S, C, CMD, E>) {
        Self::start_inner(machine, context, init, Some(now_ms), scheduler)
    }

    fn start_inner(
        machine: &'a Machine<S, E, C, CMD>,
        context: &C,
        init: &E,
        now_ms: Option<u64>,
        scheduler: impl HostScheduler<S, H>,
    ) -> (Self, StepOutcome<S, C, CMD, E>) {
        let outcome = machine.start(context, init);
        let state = MachineState {
            configuration: outcome.configuration.clone(),
            history: outcome.history.clone(),
            context: outcome.context.clone(),
            armed: outcome.armed.clone(),
        };
        let mut driven = Self {
            machine,
            state,
            timers: HostTimers::new(),
            ops: Vec::new(),
        };
        driven.settle_timers(&outcome.timers, now_ms, scheduler);
        (driven, outcome)
    }

    /// Wrap an existing [`MachineState`] without re-running entry actions.
    #[must_use]
    pub fn from_state(machine: &'a Machine<S, E, C, CMD>, state: MachineState<S, C>) -> Self {
        Self {
            machine,
            state,
            timers: HostTimers::new(),
            ops: Vec::new(),
        }
    }

    /// Restore persisted timers onto `scheduler` with remaining delays relative to `now_ms`.
    pub fn restore_timers(
        &mut self,
        persisted: &[PersistedTimer<S>],
        now_ms: u64,
        scheduler: impl HostScheduler<S, H>,
    ) {
        self.timers
            .restore_remaining(persisted, now_ms, &mut self.ops);
        self.run_ops(scheduler);
    }

    /// Step the machine with `event` and reconcile host timers via `scheduler`.
    pub fn step(
        &mut self,
        event: &E,
        scheduler: impl HostScheduler<S, H>,
    ) -> StepOutcome<S, C, CMD, E> {
        self.step_inner(event, None, scheduler)
    }

    /// Step the machine with `event`, stamping new timers with `now_ms`.
    pub fn step_at(
        &mut self,
        event: &E,
        now_ms: u64,
        scheduler: impl HostScheduler<S, H>,
    ) -> StepOutcome<S, C, CMD, E> {
        self.step_inner(event, Some(now_ms), scheduler)
    }

    fn step_inner(
        &mut self,
        event: &E,
        now_ms: Option<u64>,
        scheduler: impl HostScheduler<S, H>,
    ) -> StepOutcome<S, C, CMD, E> {
        let outcome = self.machine.step_state(&self.state, event);
        self.apply_outcome(&outcome, now_ms, scheduler);
        outcome
    }

    /// Deliver a host timer callback identified by `token`.
    ///
    /// Returns `None` if `token` is stale (its timer was already cancelled or re-armed).
    pub fn fire(
        &mut self,
        token: TimerToken,
        event: &E,
        scheduler: impl HostScheduler<S, H>,
    ) -> Option<StepOutcome<S, C, CMD, E>> {
        self.fire_inner(token, event, None, scheduler)
    }

    /// Same as [`fire`](Self::fire), stamping newly armed timers with `now_ms`.
    pub fn fire_at(
        &mut self,
        token: TimerToken,
        event: &E,
        now_ms: u64,
        scheduler: impl HostScheduler<S, H>,
    ) -> Option<StepOutcome<S, C, CMD, E>> {
        self.fire_inner(token, event, Some(now_ms), scheduler)
    }

    fn fire_inner(
        &mut self,
        token: TimerToken,
        event: &E,
        now_ms: Option<u64>,
        scheduler: impl HostScheduler<S, H>,
    ) -> Option<StepOutcome<S, C, CMD, E>> {
        let fired = self.timers.fired(token)?;
        let outcome = self.machine.step_timer(&self.state, &fired, event);
        self.apply_outcome(&outcome, now_ms, scheduler);
        Some(outcome)
    }

    fn apply_outcome(
        &mut self,
        outcome: &StepOutcome<S, C, CMD, E>,
        now_ms: Option<u64>,
        scheduler: impl HostScheduler<S, H>,
    ) {
        self.state = MachineState {
            configuration: outcome.configuration.clone(),
            history: outcome.history.clone(),
            context: outcome.context.clone(),
            armed: outcome.armed.clone(),
        };
        self.settle_timers(&outcome.timers, now_ms, scheduler);
    }

    fn settle_timers(
        &mut self,
        intents: &[TimerIntent<S>],
        now_ms: Option<u64>,
        scheduler: impl HostScheduler<S, H>,
    ) {
        match now_ms {
            Some(now) => self.timers.apply_at(intents, now, &mut self.ops),
            None => self.timers.apply(intents, &mut self.ops),
        }
        self.run_ops(scheduler);
    }

    fn run_ops(&mut self, mut scheduler: impl HostScheduler<S, H>) {
        for op in self.ops.drain(..) {
            match op {
                TimerOp::Arm {
                    state,
                    duration,
                    token,
                } => {
                    let handle = scheduler.schedule(&state, duration, token);
                    if let Some(stale) = self.timers.armed(token, handle) {
                        scheduler.cancel(stale);
                    }
                }
                TimerOp::Disarm(handle) => scheduler.cancel(handle),
            }
        }
    }

    /// Current machine state.
    #[must_use]
    pub fn state(&self) -> &MachineState<S, C> {
        &self.state
    }

    /// Pending host timers.
    #[must_use]
    pub fn timers(&self) -> &HostTimers<S, H> {
        &self.timers
    }

    /// Snapshot pending timers with their arming timestamps.
    #[must_use]
    pub fn persist_timers(&self) -> Vec<PersistedTimer<S>> {
        self.timers.persist()
    }
}
