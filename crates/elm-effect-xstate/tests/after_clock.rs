use elm_effect::{Duration, TestClock};
use elm_effect_xstate::prelude::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum Phase {
    Idle,
    Waiting,
    TimedOut,
}

#[derive(Debug, Clone)]
enum Signal {
    Start,
}

#[derive(Debug, Clone, Default)]
struct Count {
    fired: u32,
}

struct TimerHost {
    armed: Option<(Phase, Duration)>,
    elapsed_ms: u64,
}

impl TimerHost {
    fn new() -> Self {
        Self {
            armed: None,
            elapsed_ms: 0,
        }
    }

    fn absorb(&mut self, outcome: &StepOutcome<Phase, Count, (), Signal>) {
        for intent in &outcome.timers {
            match intent {
                TimerIntent::Start { state, duration } => {
                    self.armed = Some((*state, *duration));
                }
                TimerIntent::Cancel { state, .. } => {
                    if self.armed.is_some_and(|(armed, _)| armed == *state) {
                        self.armed = None;
                    }
                }
            }
        }
    }

    fn advance(&mut self, clock: &TestClock, by: Duration) -> Option<TimerFired<Phase>> {
        clock.adjust(by);
        self.elapsed_ms += by.as_millis();
        let (state, deadline) = self.armed?;
        (self.elapsed_ms >= deadline.as_millis()).then(|| TimerFired::new(state, deadline))
    }
}

#[test]
fn after_fires_on_virtual_time_without_sleeping() {
    let clock = TestClock::new();
    let machine: Machine<Phase, Signal, Count, ()> =
        Machine::builder(Phase::Idle, Count::default())
            .state(Phase::Idle, |node| {
                let start = node
                    .transition(Phase::Waiting)
                    .on(|event| matches!(event, Signal::Start));
                node.on(start)
            })
            .state(Phase::Waiting, |node| {
                let timeout = node.transition(Phase::TimedOut);
                node.after(Duration::from_millis(500), timeout)
            })
            .state(Phase::TimedOut, |node| {
                node.entry(|context, _| context.fired += 1)
            })
            .build();

    let started = machine.step(&Phase::Idle, &Count::default(), &Signal::Start);
    let mut host = TimerHost::new();
    host.absorb(&started);
    assert_eq!(
        host.armed,
        Some((Phase::Waiting, Duration::from_millis(500)))
    );

    assert!(host.advance(&clock, Duration::from_millis(499)).is_none());
    let fired = host
        .advance(&clock, Duration::from_millis(1))
        .expect("the 500ms deadline elapsed");

    let outcome = machine.step_timer(
        &MachineState::new(started.configuration, started.context),
        &fired,
        &Signal::Start,
    );
    assert_eq!(outcome.state(), Some(&Phase::TimedOut));
    assert_eq!(outcome.context.fired, 1);
}
