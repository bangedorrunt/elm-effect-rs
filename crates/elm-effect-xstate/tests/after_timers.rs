use elm_effect::Duration;
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
    Cancel,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Trace {
    log: Vec<String>,
}

const TIMEOUT: Duration = Duration::from_millis(500);

fn machine() -> Machine<Phase, Signal, Trace, ()> {
    Machine::builder(Phase::Idle, Trace::default())
        .state(Phase::Idle, |node| {
            let start = node
                .transition(Phase::Waiting)
                .on(|e| matches!(e, Signal::Start));
            node.on(start)
        })
        .state(Phase::Waiting, |node| {
            let timeout = node.transition(Phase::TimedOut);
            let cancel = node
                .transition(Phase::Idle)
                .on(|e| matches!(e, Signal::Cancel));
            node.entry(|ctx, _| ctx.log.push("enter_waiting".into()))
                .exit(|ctx, _| ctx.log.push("exit_waiting".into()))
                .after(TIMEOUT, timeout)
                .on(cancel)
        })
        .state(Phase::TimedOut, |node| {
            node.entry(|ctx, _| ctx.log.push("timed_out".into()))
        })
        .build()
}

fn run_to_waiting() -> (
    Machine<Phase, Signal, Trace, ()>,
    MachineState<Phase, Trace>,
) {
    let machine = machine();
    let entered = machine.enter(&Trace::default());
    let state = MachineState::new(entered.configuration, entered.context);
    let started = machine.step_state(&state, &Signal::Start);
    (
        machine,
        MachineState {
            configuration: started.configuration,
            history: started.history,
            context: started.context,
            armed: started.armed,
        },
    )
}

#[test]
fn entering_a_delayed_state_arms_the_timer() {
    let (_, waiting) = run_to_waiting();
    let machine = machine();

    let entered = machine.enter(&Trace::default());
    assert!(entered.timers.is_empty());

    let started = machine.step_state(
        &MachineState::new(entered.configuration, entered.context),
        &Signal::Start,
    );
    assert_eq!(
        started.timers,
        [TimerIntent::Start {
            state: Phase::Waiting,
            duration: TIMEOUT
        }]
    );
    assert_eq!(waiting.state(), Some(&Phase::Waiting));
}

#[test]
fn a_fired_timer_takes_the_after_transition() {
    let (machine, waiting) = run_to_waiting();

    let outcome = machine.step_timer(
        &waiting,
        &TimerFired::new(Phase::Waiting, TIMEOUT),
        &Signal::Start,
    );

    assert_eq!(outcome.state(), Some(&Phase::TimedOut));
    assert!(outcome.transitioned);
    assert_eq!(
        outcome.context.log,
        ["enter_waiting", "exit_waiting", "timed_out"]
    );
}

#[test]
fn leaving_a_delayed_state_cancels_it_and_a_stale_fire_does_nothing() {
    let (machine, waiting) = run_to_waiting();

    let cancelled = machine.step_state(&waiting, &Signal::Cancel);
    assert_eq!(cancelled.state(), Some(&Phase::Idle));
    assert_eq!(
        cancelled.timers,
        [TimerIntent::Cancel {
            state: Phase::Waiting,
            duration: TIMEOUT
        }]
    );

    let idle = MachineState {
        configuration: cancelled.configuration,
        history: cancelled.history,
        context: cancelled.context,
        armed: cancelled.armed,
    };
    let stale = machine.step_timer(
        &idle,
        &TimerFired::new(Phase::Waiting, TIMEOUT),
        &Signal::Start,
    );
    assert_eq!(stale.state(), Some(&Phase::Idle));
    assert!(!stale.transitioned);
}
