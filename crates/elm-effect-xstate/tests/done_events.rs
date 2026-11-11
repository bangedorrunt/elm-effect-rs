use elm_effect_xstate::prelude::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum Task {
    Work,
    StepOne,
    StepDone,
    Finished,
}

#[derive(Debug, Clone)]
enum Signal {
    Finish,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Trace {
    log: Vec<String>,
}

#[test]
fn a_final_child_completes_its_compound_and_takes_on_done() {
    let machine: Machine<Task, Signal, Trace, ()> = Machine::builder(Task::Work, Trace::default())
        .compound(Task::Work, Task::StepOne, |node| {
            let finished = node.transition(Task::Finished).label("done");
            node.on_done(finished)
        })
        .state(Task::StepOne, |node| {
            let finish = node
                .transition(Task::StepDone)
                .on(|e| matches!(e, Signal::Finish));
            node.with_parent(Task::Work)
                .entry(|ctx, _| ctx.log.push("enter_step".into()))
                .on(finish)
        })
        .final_state(Task::StepDone, |node| {
            node.with_parent(Task::Work)
                .entry(|ctx, _| ctx.log.push("enter_final".into()))
        })
        .state(Task::Finished, |node| {
            node.entry(|ctx, _| ctx.log.push("enter_finished".into()))
        })
        .build();

    let entered = machine.enter(&Trace::default());
    let outcome = machine.step_state(
        &MachineState::new(entered.configuration, entered.context),
        &Signal::Finish,
    );

    assert_eq!(outcome.state(), Some(&Task::Finished));
    assert_eq!(
        outcome.context.log,
        ["enter_step", "enter_final", "enter_finished"]
    );
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum Cell {
    Root,
    Left,
    LeftA,
    LeftDone,
    Right,
    RightA,
    RightDone,
    Finished,
}

#[test]
fn a_parallel_node_completes_in_the_step_its_last_region_finishes() {
    let machine: Machine<Cell, Signal, Trace, ()> = Machine::builder(Cell::Root, Trace::default())
        .parallel(Cell::Root, |node| {
            let done = node.transition(Cell::Finished).label("done");
            node.on_done(done)
        })
        .compound(Cell::Left, Cell::LeftA, |node| node.with_parent(Cell::Root))
        .state(Cell::LeftA, |node| {
            let finish = node
                .transition(Cell::LeftDone)
                .on(|e| matches!(e, Signal::Finish));
            node.with_parent(Cell::Left).on(finish)
        })
        .final_state(Cell::LeftDone, |node| node.with_parent(Cell::Left))
        .compound(Cell::Right, Cell::RightA, |node| {
            node.with_parent(Cell::Root)
        })
        .state(Cell::RightA, |node| {
            let finish = node
                .transition(Cell::RightDone)
                .on(|e| matches!(e, Signal::Finish));
            node.with_parent(Cell::Right).on(finish)
        })
        .final_state(Cell::RightDone, |node| node.with_parent(Cell::Right))
        .state(Cell::Finished, |node| node)
        .build();

    let entered = machine.enter(&Trace::default());
    let finished = machine.step_state(
        &MachineState::new(entered.configuration, entered.context),
        &Signal::Finish,
    );

    assert_eq!(
        finished.configuration.leaves().copied().collect::<Vec<_>>(),
        [Cell::Finished]
    );
}

#[test]
fn a_top_level_final_completes_the_machine() {
    let machine: Machine<Task, Signal, Trace, ()> = Machine::builder(Task::Work, Trace::default())
        .state(Task::Work, |node| {
            let finish = node
                .transition(Task::StepDone)
                .on(|e| matches!(e, Signal::Finish));
            node.on(finish)
        })
        .final_state(Task::StepDone, |node| node)
        .build();

    let entered = machine.enter(&Trace::default());
    assert!(!entered.done);
    let outcome = machine.step_state(
        &MachineState::new(entered.configuration, entered.context),
        &Signal::Finish,
    );

    assert_eq!(outcome.state(), Some(&Task::StepDone));
    assert!(outcome.done);
}

#[test]
fn root_entry_and_exit_surround_the_machine_lifecycle() {
    let machine: Machine<Task, Signal, Trace, ()> = Machine::builder(Task::Work, Trace::default())
        .root_entry(|ctx, _| ctx.log.push("enter_root".into()))
        .root_exit(|ctx, _| ctx.log.push("exit_root".into()))
        .state(Task::Work, |node| {
            let finish = node
                .transition(Task::StepDone)
                .on(|e| matches!(e, Signal::Finish));
            node.entry(|ctx, _| ctx.log.push("enter_work".into()))
                .exit(|ctx, _| ctx.log.push("exit_work".into()))
                .on(finish)
        })
        .final_state(Task::StepDone, |node| {
            node.entry(|ctx, _| ctx.log.push("enter_final".into()))
                .exit(|ctx, _| ctx.log.push("exit_final".into()))
        })
        .build();

    let entered = machine.start(&Trace::default(), &Signal::Finish);
    assert_eq!(entered.context.log, ["enter_root", "enter_work"]);
    let outcome = machine.step_state(
        &MachineState::new(entered.configuration, entered.context),
        &Signal::Finish,
    );

    assert!(outcome.done);
    assert_eq!(
        outcome.context.log,
        [
            "enter_root",
            "enter_work",
            "exit_work",
            "enter_final",
            "exit_final",
            "exit_root"
        ]
    );

    let replayed = machine.step_state(
        &MachineState::new(outcome.configuration, outcome.context),
        &Signal::Finish,
    );
    assert_eq!(
        replayed.context.log,
        [
            "enter_root",
            "enter_work",
            "exit_work",
            "enter_final",
            "exit_final",
            "exit_root"
        ]
    );
}

#[test]
fn a_machine_whose_root_is_final_reports_done() {
    let machine: Machine<Task, Signal, Trace, ()> =
        Machine::builder(Task::StepDone, Trace::default())
            .final_state(Task::StepDone, |node| node)
            .build();

    let entered = machine.enter(&Trace::default());
    assert!(entered.done);
}
