use elm_effect_xstate::prelude::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum Region {
    Root,
    Left,
    LeftA,
    LeftB,
    Right,
    RightA,
    RightB,
}

#[derive(Debug, Clone)]
enum Signal {
    Next,
    LeftOnly,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Trace {
    log: Vec<String>,
}

fn machine() -> Machine<Region, Signal, Trace, ()> {
    Machine::builder(Region::Root, Trace::default())
        .parallel(Region::Root, |node| {
            node.entry(|ctx, _| ctx.log.push("enter_root".into()))
        })
        .compound(Region::Left, Region::LeftA, |node| {
            node.with_parent(Region::Root)
                .entry(|ctx, _| ctx.log.push("enter_left".into()))
        })
        .state(Region::LeftA, |node| {
            let next = node
                .transition(Region::LeftB)
                .on(|e| matches!(e, Signal::Next | Signal::LeftOnly))
                .label("NEXT")
                .assign(|ctx, _| ctx.log.push("left_a".into()));
            node.with_parent(Region::Left)
                .entry(|ctx, _| ctx.log.push("enter_left_a".into()))
                .exit(|ctx, _| ctx.log.push("exit_left_a".into()))
                .on(next)
        })
        .state(Region::LeftB, |node| {
            node.with_parent(Region::Left)
                .entry(|ctx, _| ctx.log.push("enter_left_b".into()))
        })
        .compound(Region::Right, Region::RightA, |node| {
            node.with_parent(Region::Root)
                .entry(|ctx, _| ctx.log.push("enter_right".into()))
        })
        .state(Region::RightA, |node| {
            let next = node
                .transition(Region::RightB)
                .on(|e| matches!(e, Signal::Next))
                .assign(|ctx, _| ctx.log.push("right_a".into()));
            node.with_parent(Region::Right)
                .entry(|ctx, _| ctx.log.push("enter_right_a".into()))
                .exit(|ctx, _| ctx.log.push("exit_right_a".into()))
                .on(next)
        })
        .state(Region::RightB, |node| {
            node.with_parent(Region::Right)
                .entry(|ctx, _| ctx.log.push("enter_right_b".into()))
        })
        .build()
}

#[test]
fn entering_a_parallel_node_activates_every_region() {
    let machine = machine();
    let entered = machine.enter(&Trace::default());

    assert_eq!(
        entered.configuration.leaves().copied().collect::<Vec<_>>(),
        [Region::LeftA, Region::RightA]
    );
    assert_eq!(
        entered.context.log,
        [
            "enter_root",
            "enter_left",
            "enter_left_a",
            "enter_right",
            "enter_right_a"
        ]
    );
    assert!(!entered.done);
}

#[test]
fn one_event_advances_every_region_that_handles_it() {
    let machine = machine();
    let entered = machine.enter(&Trace::default());
    let state = MachineState::new(entered.configuration, entered.context);

    let outcome = machine.step_state(&state, &Signal::Next);

    assert_eq!(
        outcome.configuration.leaves().copied().collect::<Vec<_>>(),
        [Region::LeftB, Region::RightB]
    );
    assert_eq!(
        outcome.context.log,
        [
            "enter_root",
            "enter_left",
            "enter_left_a",
            "enter_right",
            "enter_right_a",
            "exit_right_a",
            "exit_left_a",
            "left_a",
            "right_a",
            "enter_left_b",
            "enter_right_b",
        ]
    );
    assert!(outcome.transitioned);
}

#[test]
fn a_region_that_ignores_the_event_keeps_its_state() {
    let machine = machine();
    let entered = machine.enter(&Trace::default());
    let state = MachineState::new(entered.configuration, entered.context);

    let outcome = machine.step_state(&state, &Signal::LeftOnly);

    assert_eq!(
        outcome.configuration.leaves().copied().collect::<Vec<_>>(),
        [Region::LeftB, Region::RightA]
    );
    assert!(outcome.context.log.contains(&"enter_right_a".to_string()));
    assert!(!outcome.context.log.contains(&"right_a".to_string()));
}

#[test]
fn the_primary_leaf_is_stable_and_the_step_convenience_shares_the_path() {
    let machine = machine();
    let entered = machine.enter(&Trace::default());

    assert_eq!(entered.current_state, Region::LeftA);

    let outcome = machine.step(&Region::Root, &Trace::default(), &Signal::Next);
    assert_eq!(
        outcome.configuration.leaves().copied().collect::<Vec<_>>(),
        [Region::LeftB, Region::RightB]
    );
}
