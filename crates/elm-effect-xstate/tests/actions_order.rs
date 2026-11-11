use elm_effect_xstate::prelude::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum TestState {
    A,
    B,
}

#[derive(Debug, Clone)]
enum TestEvent {
    Step,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct TestContext {
    log: Vec<String>,
}

#[test]
fn transition_follows_strict_l1_execution_order() {
    let machine: Machine<TestState, TestEvent, TestContext, ()> =
        Machine::builder(TestState::A, TestContext::default())
            .state(TestState::A, |node| {
                let to_b = node
                    .transition(TestState::B)
                    .on(|e| matches!(e, TestEvent::Step))
                    .action(|ctx, _, _| ctx.log.push("action_A_to_B".into()))
                    .assign(|ctx, _| ctx.log.push("assign_ctx".into()));

                node.entry(|ctx, _| ctx.log.push("enter_A".into()))
                    .exit(|ctx, _| ctx.log.push("exit_A".into()))
                    .on(to_b)
            })
            .state(TestState::B, |node| {
                node.entry(|ctx, _| ctx.log.push("enter_B".into()))
                    .exit(|ctx, _| ctx.log.push("exit_B".into()))
            })
            .build();

    let outcome = machine.step(&TestState::A, &TestContext::default(), &TestEvent::Step);

    assert_eq!(outcome.current_state, TestState::B);
    assert_eq!(
        outcome.context.log,
        vec![
            "exit_A".to_string(),
            "action_A_to_B".to_string(),
            "assign_ctx".to_string(),
            "enter_B".to_string(),
        ]
    );
    assert!(outcome.transitioned);
}

#[test]
fn reentering_self_transition_exits_and_reenters() {
    let machine: Machine<TestState, TestEvent, TestContext, ()> =
        Machine::builder(TestState::A, TestContext::default())
            .state(TestState::A, |node| {
                let to_a = node
                    .transition(TestState::A)
                    .reenter()
                    .on(|e| matches!(e, TestEvent::Step))
                    .action(|ctx, _, _| ctx.log.push("action_self".into()));

                node.entry(|ctx, _| ctx.log.push("enter_A".into()))
                    .exit(|ctx, _| ctx.log.push("exit_A".into()))
                    .on(to_a)
            })
            .build();

    let outcome = machine.step(&TestState::A, &TestContext::default(), &TestEvent::Step);

    assert_eq!(outcome.current_state, TestState::A);
    assert_eq!(
        outcome.context.log,
        vec![
            "exit_A".to_string(),
            "action_self".to_string(),
            "enter_A".to_string(),
        ]
    );
    assert!(outcome.transitioned);
}

#[test]
fn start_processes_events_raised_by_entry_actions() {
    let machine: Machine<TestState, TestEvent, TestContext, ()> =
        Machine::builder(TestState::A, TestContext::default())
            .state(TestState::A, |node| {
                let to_b = node
                    .transition(TestState::B)
                    .on(|e| matches!(e, TestEvent::Step));
                node.entry(|_, enq| enq.raise(TestEvent::Step)).on(to_b)
            })
            .state(TestState::B, |node| {
                node.entry(|ctx, _| ctx.log.push("enter_B".into()))
            })
            .build();

    let outcome = machine.start(&TestContext::default(), &TestEvent::Step);

    assert_eq!(outcome.current_state, TestState::B);
    assert_eq!(outcome.context.log, vec!["enter_B".to_string()]);
    assert!(outcome.transitioned);
}

#[test]
fn configuration_guard_checks_the_active_configuration() {
    let machine: Machine<TestState, TestEvent, TestContext, ()> =
        Machine::builder(TestState::A, TestContext::default())
            .state(TestState::A, |node| {
                let blocked = node
                    .transition(TestState::B)
                    .on(|e| matches!(e, TestEvent::Step))
                    .configuration_guard(|configuration| configuration.contains(&TestState::B));
                node.on(blocked)
            })
            .state(TestState::B, |node| node)
            .build();

    let outcome = machine.step(&TestState::A, &TestContext::default(), &TestEvent::Step);

    assert_eq!(outcome.current_state, TestState::A);
    assert!(!outcome.transitioned);
}

#[test]
fn targetless_transition_runs_actions_without_exiting_or_entering() {
    let machine: Machine<TestState, TestEvent, TestContext, ()> =
        Machine::builder(TestState::A, TestContext::default())
            .state(TestState::A, |node| {
                let targetless = node
                    .targetless()
                    .on(|e| matches!(e, TestEvent::Step))
                    .action(|ctx, _, _| ctx.log.push("action_targetless".into()));

                node.entry(|ctx, _| ctx.log.push("enter_A".into()))
                    .exit(|ctx, _| ctx.log.push("exit_A".into()))
                    .on(targetless)
            })
            .build();

    let outcome = machine.step(&TestState::A, &TestContext::default(), &TestEvent::Step);

    assert_eq!(outcome.current_state, TestState::A);
    assert_eq!(outcome.context.log, vec!["action_targetless".to_string()]);
    assert!(outcome.transitioned);
}
