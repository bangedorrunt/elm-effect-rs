use elm_effect_xstate::prelude::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum HierState {
    Active,
    ActiveIdle,
    ActiveRunning,
    Stopped,
}

#[derive(Debug, Clone)]
enum HierEvent {
    Run,
    Stop,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct HierContext {
    log: Vec<String>,
}

#[test]
fn compound_state_resolves_initial_and_bubbles_unhandled_events() {
    let machine: Machine<HierState, HierEvent, HierContext, ()> =
        Machine::builder(HierState::Active, HierContext::default())
            .compound(HierState::Active, HierState::ActiveIdle, |node| {
                let to_stopped = node
                    .transition(HierState::Stopped)
                    .on(|e| matches!(e, HierEvent::Stop));

                node.entry(|ctx, _| ctx.log.push("enter_Active".into()))
                    .exit(|ctx, _| ctx.log.push("exit_Active".into()))
                    .on(to_stopped)
            })
            .state(HierState::ActiveIdle, |node| {
                let to_running = node
                    .transition(HierState::ActiveRunning)
                    .on(|e| matches!(e, HierEvent::Run));

                node.with_parent(HierState::Active)
                    .entry(|ctx, _| ctx.log.push("enter_ActiveIdle".into()))
                    .exit(|ctx, _| ctx.log.push("exit_ActiveIdle".into()))
                    .on(to_running)
            })
            .state(HierState::ActiveRunning, |node| {
                node.with_parent(HierState::Active)
                    .entry(|ctx, _| ctx.log.push("enter_ActiveRunning".into()))
                    .exit(|ctx, _| ctx.log.push("exit_ActiveRunning".into()))
            })
            .state(HierState::Stopped, |node| {
                node.entry(|ctx, _| ctx.log.push("enter_Stopped".into()))
            })
            .build();

    assert_eq!(
        machine.resolve_leaf(&HierState::Active),
        HierState::ActiveIdle
    );

    let outcome1 = machine.step(
        &HierState::ActiveIdle,
        &HierContext::default(),
        &HierEvent::Run,
    );
    assert_eq!(outcome1.current_state, HierState::ActiveRunning);
    assert_eq!(
        outcome1.context.log,
        vec![
            "exit_ActiveIdle".to_string(),
            "enter_ActiveRunning".to_string()
        ]
    );

    let outcome2 = machine.step(
        &HierState::ActiveRunning,
        &HierContext::default(),
        &HierEvent::Stop,
    );
    assert_eq!(outcome2.current_state, HierState::Stopped);
    assert_eq!(
        outcome2.context.log,
        vec![
            "exit_ActiveRunning".to_string(),
            "exit_Active".to_string(),
            "enter_Stopped".to_string(),
        ]
    );
}
