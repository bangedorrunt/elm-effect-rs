use elm_effect_xstate::prelude::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum Spot {
    Root,
    Child,
}

#[derive(Debug, Clone)]
enum Signal {
    Refresh,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Trace {
    log: Vec<String>,
}

fn self_machine(reenter: bool) -> Machine<Spot, Signal, Trace, ()> {
    Machine::builder(Spot::Root, Trace::default())
        .state(Spot::Root, |node| {
            let refresh = node
                .transition(Spot::Root)
                .reenter_with(reenter)
                .on(|event| matches!(event, Signal::Refresh));
            node.entry(|trace, _| trace.log.push("enter".into()))
                .exit(|trace, _| trace.log.push("exit".into()))
                .on(refresh)
        })
        .build()
}

#[test]
fn a_self_transition_defaults_to_no_reentry() {
    let machine = self_machine(false);
    let entered = machine.enter(&Trace::default());
    let outcome = machine.step_state(
        &MachineState::new(entered.configuration, entered.context),
        &Signal::Refresh,
    );

    assert_eq!(outcome.current_state, Spot::Root);
    assert_eq!(outcome.context.log, ["enter"]);
    assert!(outcome.transitioned);
}

#[test]
fn reenter_true_exits_and_reenters_the_source() {
    let machine = self_machine(true);
    let entered = machine.enter(&Trace::default());
    let outcome = machine.step_state(
        &MachineState::new(entered.configuration, entered.context),
        &Signal::Refresh,
    );

    assert_eq!(outcome.current_state, Spot::Root);
    assert_eq!(outcome.context.log, ["enter", "exit", "enter"]);
    assert!(outcome.transitioned);
}

#[test]
fn a_non_reentering_child_target_keeps_its_parent_active() {
    let machine: Machine<Spot, Signal, Trace, ()> = Machine::builder(Spot::Root, Trace::default())
        .compound(Spot::Root, Spot::Child, |node| {
            node.entry(|trace, _| trace.log.push("enter_root".into()))
                .exit(|trace, _| trace.log.push("exit_root".into()))
        })
        .state(Spot::Child, |node| {
            let refresh = node
                .transition(Spot::Child)
                .on(|event| matches!(event, Signal::Refresh));
            node.with_parent(Spot::Root)
                .entry(|trace, _| trace.log.push("enter_child".into()))
                .exit(|trace, _| trace.log.push("exit_child".into()))
                .on(refresh)
        })
        .build();

    let entered = machine.enter(&Trace::default());
    let outcome = machine.step_state(
        &MachineState::new(entered.configuration, entered.context),
        &Signal::Refresh,
    );

    assert_eq!(outcome.context.log, ["enter_root", "enter_child"]);
}

#[test]
fn a_non_reentering_compound_self_target_resets_its_child() {
    let machine: Machine<Spot, Signal, Trace, ()> = Machine::builder(Spot::Root, Trace::default())
        .compound(Spot::Root, Spot::Child, |node| {
            let refresh = node
                .transition(Spot::Root)
                .on(|event| matches!(event, Signal::Refresh));
            node.entry(|trace, _| trace.log.push("enter_root".into()))
                .exit(|trace, _| trace.log.push("exit_root".into()))
                .on(refresh)
        })
        .state(Spot::Child, |node| {
            node.with_parent(Spot::Root)
                .entry(|trace, _| trace.log.push("enter_child".into()))
                .exit(|trace, _| trace.log.push("exit_child".into()))
        })
        .build();

    let entered = machine.enter(&Trace::default());
    let outcome = machine.step_state(
        &MachineState::new(entered.configuration, entered.context),
        &Signal::Refresh,
    );

    assert_eq!(
        outcome.context.log,
        ["enter_root", "enter_child", "exit_child", "enter_child"]
    );
    assert_eq!(outcome.state(), Some(&Spot::Child));
}

#[test]
fn a_reentering_child_target_exits_and_reenters_the_child() {
    let machine: Machine<Spot, Signal, Trace, ()> = Machine::builder(Spot::Root, Trace::default())
        .compound(Spot::Root, Spot::Child, |node| {
            node.entry(|trace, _| trace.log.push("enter_root".into()))
                .exit(|trace, _| trace.log.push("exit_root".into()))
        })
        .state(Spot::Child, |node| {
            let refresh = node
                .transition(Spot::Child)
                .reenter()
                .on(|event| matches!(event, Signal::Refresh));
            node.with_parent(Spot::Root)
                .entry(|trace, _| trace.log.push("enter_child".into()))
                .exit(|trace, _| trace.log.push("exit_child".into()))
                .on(refresh)
        })
        .build();

    let entered = machine.enter(&Trace::default());
    let outcome = machine.step_state(
        &MachineState::new(entered.configuration, entered.context),
        &Signal::Refresh,
    );

    assert_eq!(
        outcome.context.log,
        ["enter_root", "enter_child", "exit_child", "enter_child"]
    );
}
