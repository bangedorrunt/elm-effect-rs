use elm_effect_xstate::Topology;
use elm_effect_xstate::prelude::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum S {
    Start,
    Work,
    Loop,
    Done,
    Orphan,
    Group,
    GroupA,
    GroupB,
    Both,
    Left,
    Right,
}

#[derive(Debug, Clone)]
enum E {
    Go,
    Stop,
}

type M = Machine<S, E, (), ()>;

fn go(e: &E) -> bool {
    matches!(e, E::Go)
}

fn stop(e: &E) -> bool {
    matches!(e, E::Stop)
}

fn flat(with_exit: bool) -> M {
    Machine::builder(S::Start, ())
        .state(S::Start, |n| {
            let t = n.transition(S::Work).on(go);
            n.on(t)
        })
        .state(S::Work, |n| {
            let to_loop = n.transition(S::Loop).on(go);
            let done = n.transition(S::Done).on(stop);
            n.on(to_loop).on(done)
        })
        .state(S::Loop, |n| {
            let back = n.transition(S::Loop).on(go);
            if with_exit {
                let out = n.transition(S::Done).on(stop);
                n.on(back).on(out)
            } else {
                n.on(back)
            }
        })
        .final_state(S::Done, |n| n)
        .state(S::Orphan, |n| {
            let t = n.transition(S::Done).on(stop);
            n.on(t)
        })
        .build()
}

#[test]
fn unreachable_names_a_state_no_edge_enters() {
    let machine = flat(true);
    let topo: Topology<S> = machine.topology();
    assert_eq!(topo.unreachable(), vec![S::Orphan]);
    let work = machine.step(&S::Start, &(), &E::Go).current_state;
    assert_eq!(machine.step(&work, &(), &E::Stop).current_state, S::Done);
}

#[test]
fn cannot_finish_names_a_reachable_leaf_with_no_path_to_a_final() {
    assert_eq!(flat(false).topology().cannot_finish(), vec![S::Loop]);
    assert!(flat(true).topology().cannot_finish().is_empty());
}

#[test]
fn cannot_reach_takes_any_goal() {
    let topo = flat(false).topology();
    assert_eq!(
        topo.cannot_reach(|s| *s == S::Start),
        vec![S::Work, S::Loop, S::Done]
    );
}

#[test]
fn a_parent_edge_moves_every_child_and_entering_a_compound_enters_its_initial() {
    let machine: M = Machine::builder(S::Start, ())
        .state(S::Start, |n| {
            let t = n.transition(S::Group).on(go);
            n.on(t)
        })
        .compound(S::Group, S::GroupA, |n| {
            let out = n.transition(S::Done).on(stop);
            n.on(out)
        })
        .state(S::GroupA, |n| {
            let t = n.transition(S::GroupB).on(go);
            n.with_parent(S::Group).on(t)
        })
        .state(S::GroupB, |n| n.with_parent(S::Group))
        .final_state(S::Done, |n| n)
        .build();
    let topo = machine.topology();
    assert!(topo.unreachable().is_empty());
    assert!(
        topo.cannot_finish().is_empty(),
        "GroupB has no edge of its own; the parent's edge must carry it to Done"
    );
}

#[test]
fn entering_a_parallel_state_enters_every_region() {
    let machine: M = Machine::builder(S::Start, ())
        .state(S::Start, |n| {
            let t = n.transition(S::Both).on(go);
            n.on(t)
        })
        .parallel(S::Both, |n| n)
        .state(S::Left, |n| n.with_parent(S::Both))
        .state(S::Right, |n| n.with_parent(S::Both))
        .build();
    let topo = machine.topology();
    assert!(topo.unreachable().is_empty());
    assert_eq!(topo.cannot_finish(), vec![S::Start, S::Left, S::Right]);
}

#[test]
fn a_guarded_edge_counts_as_enabled() {
    let machine: M = Machine::builder(S::Start, ())
        .state(S::Start, |n| {
            let never = n.transition(S::Done).on(go).guard(|_, _| false);
            n.on(never)
        })
        .final_state(S::Done, |n| n)
        .build();
    let topo = machine.topology();
    assert!(topo.unreachable().is_empty());
    assert!(topo.cannot_finish().is_empty());
    assert_eq!(
        machine.step(&S::Start, &(), &E::Go).current_state,
        S::Start,
        "the guard blocks the step the topology still counts"
    );
}
