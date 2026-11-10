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
fn self_transition_exits_and_reenters() {
    let machine: Machine<TestState, TestEvent, TestContext, ()> =
        Machine::builder(TestState::A, TestContext::default())
            .state(TestState::A, |node| {
                let to_a = node
                    .transition(TestState::A)
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
