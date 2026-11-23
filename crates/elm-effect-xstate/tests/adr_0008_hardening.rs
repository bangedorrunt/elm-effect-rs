use elm_effect::{Duration, Effect};
use elm_effect_elm::{Boot, Cmd, Durability, Failure, Program};
use elm_effect_xstate::prelude::*;
use std::collections::BTreeSet;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
enum TopoState {
    Outer,
    Inner,
    LeafA,
    LeafB,
    Par,
    Reg1,
    R1A,
    Reg2,
    R2A,
    Standalone,
}

#[derive(Debug, Clone)]
enum TopoEv {
    Go,
}

#[test]
fn compiled_preorder_index_answers_descendant_and_ancestry_queries_in_o1() {
    let machine: Machine<TopoState, TopoEv, (), ()> = Machine::builder(TopoState::Outer, ())
        .compound(TopoState::Outer, TopoState::Inner, |n| n)
        .compound(TopoState::Inner, TopoState::LeafA, |n| {
            n.with_parent(TopoState::Outer)
        })
        .state(TopoState::LeafA, |n| {
            let tr = n
                .transition(TopoState::LeafB)
                .on(|e| matches!(e, TopoEv::Go));
            n.with_parent(TopoState::Inner).on(tr)
        })
        .state(TopoState::LeafB, |n| n.with_parent(TopoState::Inner))
        .parallel(TopoState::Par, |n| n)
        .compound(TopoState::Reg1, TopoState::R1A, |n| {
            n.with_parent(TopoState::Par)
        })
        .state(TopoState::R1A, |n| n.with_parent(TopoState::Reg1))
        .compound(TopoState::Reg2, TopoState::R2A, |n| {
            n.with_parent(TopoState::Par)
        })
        .state(TopoState::R2A, |n| n.with_parent(TopoState::Reg2))
        .state(TopoState::Standalone, |n| n)
        .build();

    for s in [
        TopoState::Outer,
        TopoState::Inner,
        TopoState::LeafA,
        TopoState::LeafB,
        TopoState::Par,
        TopoState::Reg1,
        TopoState::R1A,
        TopoState::Reg2,
        TopoState::R2A,
        TopoState::Standalone,
    ] {
        assert!(
            machine.state_idx(&s).is_some(),
            "missing state_idx for {s:?}"
        );
        assert!(
            machine.subtree_end(&s).is_some(),
            "missing subtree_end for {s:?}"
        );
    }

    assert!(machine.is_descendant(&TopoState::LeafA, &TopoState::Inner));
    assert!(machine.is_descendant(&TopoState::LeafA, &TopoState::Outer));
    assert!(machine.is_descendant(&TopoState::R2A, &TopoState::Par));
    assert!(!machine.is_descendant(&TopoState::LeafA, &TopoState::LeafA));
    assert!(!machine.is_descendant(&TopoState::Outer, &TopoState::Inner));
    assert!(!machine.is_descendant(&TopoState::Standalone, &TopoState::Outer));

    let outer_idx = machine.state_idx(&TopoState::Outer).unwrap();
    let outer_end = machine.subtree_end(&TopoState::Outer).unwrap();
    let inner_idx = machine.state_idx(&TopoState::Inner).unwrap();
    let leaf_idx = machine.state_idx(&TopoState::LeafA).unwrap();
    let standalone_idx = machine.state_idx(&TopoState::Standalone).unwrap();
    assert!(outer_idx < inner_idx && inner_idx < leaf_idx && leaf_idx < outer_end);
    assert!(standalone_idx < outer_idx || standalone_idx >= outer_end);

    let topo = machine.topology();
    assert_eq!(topo.nodes[&TopoState::LeafA].parent, Some(TopoState::Inner));
    assert_eq!(topo.nodes[&TopoState::Inner].parent, Some(TopoState::Outer));
    assert_eq!(topo.nodes[&TopoState::R2A].parent, Some(TopoState::Reg2));
    assert_eq!(topo.nodes[&TopoState::Reg2].parent, Some(TopoState::Par));
    assert_eq!(
        machine
            .step(&TopoState::LeafA, &(), &TopoEv::Go)
            .current_state,
        TopoState::LeafB
    );
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum ProbeState {
    Idle,
    Running,
    Done,
}

#[derive(Debug, Clone)]
enum ProbeEv {
    Init,
    Start,
    Finish,
}

#[derive(Debug, Clone, Default)]
struct ProbeCtx {
    ready: bool,
    runs: u32,
}

#[test]
fn machine_can_probes_enabled_transitions_without_mutating_state() {
    let machine: Machine<ProbeState, ProbeEv, ProbeCtx, ()> =
        Machine::builder(ProbeState::Idle, ProbeCtx::default())
            .state(ProbeState::Idle, |n| {
                let tr = n
                    .transition(ProbeState::Running)
                    .on(|e| matches!(e, ProbeEv::Start))
                    .guard(|ctx, _| ctx.ready)
                    .assign(|ctx, _| ctx.runs += 1);
                n.on(tr)
            })
            .state(ProbeState::Running, |n| {
                let tr = n
                    .transition(ProbeState::Done)
                    .on(|e| matches!(e, ProbeEv::Finish));
                n.on(tr)
            })
            .final_state(ProbeState::Done, |n| n)
            .build();

    let blocked = MachineState::at(
        ProbeState::Idle,
        ProbeCtx {
            ready: false,
            runs: 0,
        },
    );
    assert!(!machine.can(&blocked, &ProbeEv::Start));
    assert!(!machine.can(&blocked, &ProbeEv::Finish));

    let started = machine.start(
        &ProbeCtx {
            ready: true,
            runs: 0,
        },
        &ProbeEv::Init,
    );
    let ready = MachineState {
        configuration: started.configuration,
        history: started.history,
        context: started.context,
        armed: started.armed,
    };
    assert!(machine.can(&ready, &ProbeEv::Start));
    assert_eq!(ready.context.runs, 0);
    assert_eq!(ready.state(), Some(&ProbeState::Idle));

    let done = MachineState::at(
        ProbeState::Done,
        ProbeCtx {
            ready: true,
            runs: 1,
        },
    );
    assert!(!machine.can(&done, &ProbeEv::Start));
    assert!(!machine.can(&done, &ProbeEv::Finish));
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum RouteState {
    Mode,
    Entry,
    Fast,
    Slow,
    Fallback,
}

#[derive(Debug, Clone)]
enum RouteEv {
    Init,
    Route(u8),
}

#[test]
fn dynamic_transition_fn_selects_target_or_declines_to_bubble_to_parent() {
    let machine: Machine<RouteState, RouteEv, (), ()> = Machine::builder(RouteState::Mode, ())
        .compound(RouteState::Mode, RouteState::Entry, |n| {
            let tr = n
                .transition(RouteState::Fallback)
                .on(|e| matches!(e, RouteEv::Route(_)));
            n.on(tr)
        })
        .state(RouteState::Entry, |n| {
            let tr = n.on_fn(|_ctx, ev| match ev {
                RouteEv::Route(1) => Some(RouteState::Fast),
                RouteEv::Route(2) => Some(RouteState::Slow),
                _ => None,
            });
            n.with_parent(RouteState::Mode).on(tr)
        })
        .state(RouteState::Fast, |n| n.with_parent(RouteState::Mode))
        .state(RouteState::Slow, |n| n.with_parent(RouteState::Mode))
        .state(RouteState::Fallback, |n| n)
        .build();

    let start = machine.start(&(), &RouteEv::Init);
    assert_eq!(start.current_state, RouteState::Entry);
    assert!(
        machine.topology().nodes[&RouteState::Entry]
            .targets
            .is_empty(),
        "unresolved dynamic transition must not inject a static self-loop"
    );
    assert!(!machine.to_mermaid().contains("Entry --> Entry"));

    let s1 = machine.step(&RouteState::Entry, &(), &RouteEv::Route(1));
    assert_eq!(s1.current_state, RouteState::Fast);

    let s2 = machine.step(&RouteState::Entry, &(), &RouteEv::Route(2));
    assert_eq!(s2.current_state, RouteState::Slow);

    // Returning `None` from `on_fn` declines the event so parent `Mode` handles it.
    let s3 = machine.step(&RouteState::Entry, &(), &RouteEv::Route(99));
    assert_eq!(s3.current_state, RouteState::Fallback);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum ChoiceState {
    Start,
    Decide,
    High,
    Low,
}

#[derive(Debug, Clone)]
enum ChoiceEv {
    Eval(i32),
}

#[test]
fn choice_pseudo_state_branches_immediately_on_entry() {
    let machine: Machine<ChoiceState, ChoiceEv, i32, ()> = Machine::builder(ChoiceState::Start, 0)
        .state(ChoiceState::Start, |n| {
            let tr = n
                .transition(ChoiceState::Decide)
                .on(|e| matches!(e, ChoiceEv::Eval(_)))
                .assign(|ctx, ev| {
                    let ChoiceEv::Eval(v) = ev;
                    *ctx = *v;
                });
            n.on(tr)
        })
        .choice(ChoiceState::Decide, |n| {
            let high = n.transition(ChoiceState::High).guard(|ctx, _| *ctx >= 10);
            let low = n.transition(ChoiceState::Low);
            n.always(high).always(low)
        })
        .final_state(ChoiceState::High, |n| n)
        .final_state(ChoiceState::Low, |n| n)
        .build();

    let high = machine.step(&ChoiceState::Start, &0, &ChoiceEv::Eval(12));
    assert_eq!(high.current_state, ChoiceState::High);
    assert!(high.done);

    let low = machine.step(&ChoiceState::Start, &0, &ChoiceEv::Eval(3));
    assert_eq!(low.current_state, ChoiceState::Low);
    assert!(low.done);

    let topo = machine.topology();
    assert!(topo.unreachable().is_empty());
    assert!(topo.cannot_finish().is_empty());

    let dyn_machine = DynamicMachine::builder(
        serde_json::from_str(
            r#"{
                "initial": "Start",
                "states": {
                    "Start": {"on": {"GO_HIGH": "Decide", "GO_LOW": "Decide"}},
                    "Decide": {
                        "type": "choice",
                        "always": [
                            {"target": "High", "guard": "is_high"},
                            {"target": "Low"}
                        ]
                    },
                    "High": {},
                    "Low": {}
                }
            }"#,
        )
        .expect("choice schema"),
    )
    .guard("is_high", |_ctx, ev| ev.r#type == "GO_HIGH")
    .build();

    let s = dyn_machine.start(
        dyn_machine.initial_context(),
        &DynamicEvent::new("xstate.init"),
    );
    let out = dyn_machine.step(&s.current_state, &s.context, &DynamicEvent::new("GO_HIGH"));
    assert_eq!(out.current_state, "High");
    let out_low = dyn_machine.step(&s.current_state, &s.context, &DynamicEvent::new("GO_LOW"));
    assert_eq!(out_low.current_state, "Low");
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum NetTimerState {
    Idle,
    Transient,
    Settled,
    Timed,
}

#[derive(Debug, Clone)]
enum NetTimerEv {
    Go,
    EnterTimed,
    Reenter,
}

#[test]
fn transient_always_state_nets_intra_macrostep_timer_start_and_cancel() {
    let machine: Machine<NetTimerState, NetTimerEv, (), ()> =
        Machine::builder(NetTimerState::Idle, ())
            .state(NetTimerState::Idle, |n| {
                let to_transient = n
                    .transition(NetTimerState::Transient)
                    .on(|e| matches!(e, NetTimerEv::Go));
                let to_timed = n
                    .transition(NetTimerState::Timed)
                    .on(|e| matches!(e, NetTimerEv::EnterTimed));
                n.on(to_transient).on(to_timed)
            })
            .state(NetTimerState::Transient, |n| {
                let after_tr = n.transition(NetTimerState::Idle);
                let always_tr = n.transition(NetTimerState::Settled);
                n.after(Duration::from_millis(50), after_tr)
                    .always(always_tr)
            })
            .state(NetTimerState::Settled, |n| n)
            .state(NetTimerState::Timed, |n| {
                let after_tr = n.transition(NetTimerState::Idle);
                let reenter_tr = n
                    .transition(NetTimerState::Timed)
                    .reenter()
                    .on(|e| matches!(e, NetTimerEv::Reenter));
                n.after(Duration::from_millis(75), after_tr).on(reenter_tr)
            })
            .build();

    // Entering a transient `after` state that immediately `always`-exits emits 0 net timers.
    let out = machine.step(&NetTimerState::Idle, &(), &NetTimerEv::Go);
    assert_eq!(out.current_state, NetTimerState::Settled);
    assert!(
        out.timers.is_empty(),
        "expected netted timers, got {:?}",
        out.timers
    );
    assert!(out.armed.is_empty());

    // Explicit re-entry emits [Cancel, Start] which MUST both be kept (not netted).
    let entered = machine.step(&NetTimerState::Idle, &(), &NetTimerEv::EnterTimed);
    let timed_state = MachineState {
        configuration: entered.configuration,
        history: entered.history,
        context: entered.context,
        armed: entered.armed,
    };
    let reentered = machine.step_state(&timed_state, &NetTimerEv::Reenter);
    assert_eq!(
        reentered.timers,
        vec![
            TimerIntent::Cancel {
                state: NetTimerState::Timed,
                duration: Duration::from_millis(75),
            },
            TimerIntent::Start {
                state: NetTimerState::Timed,
                duration: Duration::from_millis(75),
            },
        ]
    );
}

#[test]
fn transient_invoke_state_nets_intra_macrostep_spawn_and_stop_d17() {
    let worker = DynamicMachine::from_json_str(r#"{"initial":"Work","states":{"Work":{}}}"#)
        .expect("worker");
    let root = DynamicMachine::builder(
        serde_json::from_str(
            r#"{
                "initial": "Idle",
                "states": {
                    "Idle": {"on": {"TRANSIENT": "Transient", "LIVE": "Running"}},
                    "Transient": {
                        "invoke": {"id": "ephemeral", "src": "worker"},
                        "always": [{"target": "Done"}]
                    },
                    "Running": {
                        "invoke": {"id": "worker_inst", "src": "worker"},
                        "on": {"RESTART": {"target": "Running", "reenter": true}}
                    },
                    "Done": {}
                }
            }"#,
        )
        .expect("config"),
    )
    .actor("worker", ActorLogic::from_machine(worker))
    .build();

    let s0 = root.start(root.initial_context(), &DynamicEvent::new("xstate.init"));
    let transient = root.step(
        &s0.current_state,
        &s0.context,
        &DynamicEvent::new("TRANSIENT"),
    );
    assert_eq!(transient.current_state, "Done");
    assert!(
        transient.directives.is_empty(),
        "transient Spawn+Stop must net to empty, got {:?}",
        transient.directives
    );

    // Re-entering an invoke state keeps [Stop, Spawn].
    let running = root.step(&s0.current_state, &s0.context, &DynamicEvent::new("LIVE"));
    let restarted = root.step(
        &running.current_state,
        &running.context,
        &DynamicEvent::new("RESTART"),
    );
    assert_eq!(restarted.directives.len(), 2);
    assert!(
        matches!(&restarted.directives[0], ActorDirective::Stop { id } if id.as_str() == "worker_inst")
    );
    assert!(
        matches!(&restarted.directives[1], ActorDirective::Spawn { id, .. } if id.as_str() == "worker_inst")
    );
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum HistState {
    Work,
    H,
    A,
    B,
    Outside,
}

#[derive(Debug, Clone)]
enum HistEv {
    Init,
    ToB,
    Leave,
    ReenterViaHistory,
}

#[test]
fn history_reentry_on_parent_restores_newly_recorded_leaf_validly_d4() {
    let machine: Machine<HistState, HistEv, (), ()> = Machine::builder(HistState::Work, ())
        .compound(HistState::Work, HistState::A, |n| {
            let leave = n
                .transition(HistState::Outside)
                .on(|e| matches!(e, HistEv::Leave));
            let reenter = n
                .transition(HistState::H)
                .reenter()
                .on(|e| matches!(e, HistEv::ReenterViaHistory));
            n.on(leave).on(reenter)
        })
        .history(HistState::H, HistState::A, |n| {
            n.with_parent(HistState::Work)
        })
        .state(HistState::A, |n| {
            let to_b = n.transition(HistState::B).on(|e| matches!(e, HistEv::ToB));
            n.with_parent(HistState::Work).on(to_b)
        })
        .state(HistState::B, |n| n.with_parent(HistState::Work))
        .state(HistState::Outside, |n| {
            let reenter = n
                .transition(HistState::H)
                .on(|e| matches!(e, HistEv::ReenterViaHistory));
            n.on(reenter)
        })
        .build();

    // Visit A -> leave to Outside (records H = [A]), then re-enter H -> A, move A -> B,
    // then transition `Work -> H` directly! Exiting B must update H = [B], and resolving
    // `Work -> H` must restore B (the newly recorded leaf), NOT stale A.
    let s0 = machine.start(&(), &HistEv::Init);
    assert_eq!(s0.current_state, HistState::A);

    let left = machine.step_state(&MachineState::new(s0.configuration, ()), &HistEv::Leave);
    assert_eq!(left.current_state, HistState::Outside);

    let back_to_a = machine.step_state(
        &MachineState {
            configuration: left.configuration,
            history: left.history,
            context: (),
            armed: left.armed,
        },
        &HistEv::ReenterViaHistory,
    );
    assert_eq!(back_to_a.current_state, HistState::A);

    let at_b = machine.step_state(
        &MachineState {
            configuration: back_to_a.configuration,
            history: back_to_a.history,
            context: (),
            armed: back_to_a.armed,
        },
        &HistEv::ToB,
    );
    assert_eq!(at_b.current_state, HistState::B);

    let reentered = machine.step_state(
        &MachineState {
            configuration: at_b.configuration,
            history: at_b.history,
            context: (),
            armed: at_b.armed,
        },
        &HistEv::ReenterViaHistory,
    );
    assert_eq!(reentered.current_state, HistState::B);
    assert_eq!(
        reentered
            .history
            .get(&HistState::H)
            .and_then(|c| c.primary()),
        Some(&HistState::B)
    );
}

#[derive(Default)]
struct RecordingScheduler {
    next_handle: usize,
    scheduled: Vec<(String, Duration, TimerToken, usize)>,
    cancelled: BTreeSet<usize>,
}

impl<S: std::fmt::Debug> HostScheduler<S, usize> for RecordingScheduler {
    fn schedule(&mut self, state: &S, delay: Duration, token: TimerToken) -> usize {
        self.next_handle += 1;
        let handle = self.next_handle;
        self.scheduled
            .push((format!("{state:?}"), delay, token, handle));
        handle
    }

    fn cancel(&mut self, handle: usize) {
        self.cancelled.insert(handle);
    }
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
enum HostState {
    Idle,
    Armed,
    Timeout,
}

#[derive(Debug, Clone)]
enum HostEv {
    Init,
    Arm,
    Cancel,
}

#[test]
fn host_timers_and_driven_machine_reject_stale_tokens_and_preserve_remaining_deadlines_d19() {
    let machine: Machine<HostState, HostEv, (), ()> = Machine::builder(HostState::Idle, ())
        .state(HostState::Idle, |n| {
            let tr = n
                .transition(HostState::Armed)
                .on(|e| matches!(e, HostEv::Arm));
            n.on(tr)
        })
        .state(HostState::Armed, |n| {
            let timeout = n.transition(HostState::Timeout);
            let cancel = n
                .transition(HostState::Idle)
                .on(|e| matches!(e, HostEv::Cancel));
            let rearm = n
                .transition(HostState::Armed)
                .reenter()
                .on(|e| matches!(e, HostEv::Arm));
            n.after(Duration::from_millis(500), timeout)
                .on(cancel)
                .on(rearm)
        })
        .state(HostState::Timeout, |n| n)
        .build();

    let mut sched = RecordingScheduler::default();
    let (mut driven, _) = DrivenMachine::start_at(&machine, &(), &HostEv::Init, 1000, &mut sched);
    assert!(driven.timers().is_empty());

    // Enter Armed at t=1000ms -> schedules token_1 with 500ms delay.
    driven.step_at(&HostEv::Arm, 1000, &mut sched);
    assert_eq!(sched.scheduled.len(), 1);
    let (_, d1, tok1, h1) = sched.scheduled[0].clone();
    assert_eq!(d1, Duration::from_millis(500));

    // Re-enter Armed (ABA for same state+duration) at t=1100ms -> cancels h1, issues tok2.
    driven.step_at(&HostEv::Arm, 1100, &mut sched);
    assert!(sched.cancelled.contains(&h1));
    assert_eq!(sched.scheduled.len(), 2);
    let (_, _, tok2, h2) = sched.scheduled[1].clone();
    assert_ne!(tok1, tok2);

    // Late callback carrying stale `tok1` must be rejected!
    assert!(
        driven
            .fire_at(tok1, &HostEv::Init, 1500, &mut sched)
            .is_none()
    );
    assert_eq!(driven.state().state(), Some(&HostState::Armed));

    // Cancel and reject `tok2` as well.
    driven.step_at(&HostEv::Cancel, 1200, &mut sched);
    assert!(sched.cancelled.contains(&h2));
    assert!(
        driven
            .fire_at(tok2, &HostEv::Init, 1600, &mut sched)
            .is_none()
    );
    assert_eq!(driven.state().state(), Some(&HostState::Idle));

    // Re-arm at t=1000ms, persist, then restore at t=1350ms -> remaining delay must be 150ms!
    driven.step_at(&HostEv::Arm, 1000, &mut sched);
    let persisted = driven.persist_timers();
    assert_eq!(persisted.len(), 1);
    let saved_state = driven.state().clone();

    let round_trip: Vec<PersistedTimer<HostState>> =
        serde_json::from_str(&serde_json::to_string(&persisted).expect("serialize"))
            .expect("deserialize");

    let mut sched2 = RecordingScheduler::default();
    let mut restored = DrivenMachine::from_state(&machine, saved_state);
    restored.restore_timers(&round_trip, 1350, &mut sched2);
    assert_eq!(sched2.scheduled.len(), 1);
    let (_, rem_delay, tok_r, _) = sched2.scheduled[0].clone();
    assert_eq!(rem_delay, Duration::from_millis(150));

    // Firing tok_r passes the original declared duration (500ms) so Machine::step_timer matches!
    let fired = restored
        .fire_at(tok_r, &HostEv::Init, 1500, &mut sched2)
        .expect("restored token must fire");
    assert_eq!(fired.current_state, HostState::Timeout);

    // Past-due restore schedules immediate Duration::ZERO catch-up.
    let mut sched3 = RecordingScheduler::default();
    let mut late_timers: HostTimers<HostState, usize> = HostTimers::new();
    let mut ops = Vec::new();
    late_timers.restore_remaining(&round_trip, 2500, &mut ops);
    assert_eq!(ops.len(), 1);
    if let TimerOp::Arm { duration, .. } = ops[0] {
        assert_eq!(duration, Duration::ZERO);
    } else {
        panic!("expected Arm op");
    }
    let _ = &mut sched3;
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
enum ElmTimerState {
    Waiting,
    Done,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum ElmTimerEvent {
    Init,
    SetDelay(u64),
    TimerTick,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct ElmTimerCtx {
    delay_ms: u64,
}

impl Default for ElmTimerCtx {
    fn default() -> Self {
        Self { delay_ms: 50 }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum ElmTimerCmd {
    Arm(ElmTimerState, u64),
}

impl Cmd for ElmTimerCmd {
    fn durability(&self) -> Durability {
        Durability::Ephemeral
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum ElmTimerMsg {
    Event(ElmTimerEvent),
    Fired { state: ElmTimerState, ms: u64 },
}

struct ElmTimerDef;

impl StatechartDefinition for ElmTimerDef {
    type State = ElmTimerState;
    type Event = ElmTimerEvent;
    type Context = ElmTimerCtx;
    type Cmd = ElmTimerCmd;
    type Msg = ElmTimerMsg;
    type Err = String;

    fn machine() -> Machine<Self::State, Self::Event, Self::Context, Self::Cmd> {
        Machine::builder(ElmTimerState::Waiting, ElmTimerCtx::default())
            .state(ElmTimerState::Waiting, |n| {
                let done_tr = n.transition(ElmTimerState::Done);
                let set_tr = n
                    .targetless()
                    .on(|e| matches!(e, ElmTimerEvent::SetDelay(_)))
                    .assign(|ctx, ev| {
                        if let ElmTimerEvent::SetDelay(ms) = ev {
                            ctx.delay_ms = *ms;
                        }
                    });
                n.after_fn(
                    "dynamic_wait",
                    |ctx| Some(Duration::from_millis(ctx.delay_ms)),
                    done_tr,
                )
                .on(set_tr)
            })
            .final_state(ElmTimerState::Done, |n| n)
            .build()
    }

    fn init_event() -> Self::Event {
        ElmTimerEvent::Init
    }

    fn extract_event(msg: &Self::Msg) -> Option<Self::Event> {
        match msg {
            ElmTimerMsg::Event(e) => Some(e.clone()),
            ElmTimerMsg::Fired { .. } => None,
        }
    }

    fn extract_timer(msg: &Self::Msg) -> Option<(TimerFired<Self::State>, Self::Event)> {
        match msg {
            ElmTimerMsg::Fired { state, ms } => Some((
                TimerFired::new(*state, Duration::from_millis(*ms)),
                ElmTimerEvent::TimerTick,
            )),
            ElmTimerMsg::Event(_) => None,
        }
    }

    fn timer_cmd(timer: &TimerIntent<Self::State>) -> Option<Self::Cmd> {
        match timer {
            TimerIntent::Start { state, duration } => {
                Some(ElmTimerCmd::Arm(*state, duration.as_millis()))
            }
            TimerIntent::Cancel { .. } => None,
        }
    }

    fn failure(_: &Failure<Self::Err>) -> Self::Msg {
        ElmTimerMsg::Event(ElmTimerEvent::Init)
    }
}

#[test]
fn statechart_program_adapter_preserves_armed_timers_and_routes_extract_timer() {
    type P = StatechartProgram<ElmTimerDef>;

    let boot = P::init(&Boot::default());
    assert_eq!(boot.model.state(), Some(&ElmTimerState::Waiting));
    assert_eq!(
        boot.commands,
        vec![ElmTimerCmd::Arm(ElmTimerState::Waiting, 50)]
    );
    assert_eq!(boot.model.armed.len(), 1);
    assert_eq!(boot.model.armed[0].duration, Duration::from_millis(50));

    // Mutating context via a targetless transition keeps `armed` intact and serializable.
    let updated = P::update(
        &boot.model,
        &ElmTimerMsg::Event(ElmTimerEvent::SetDelay(900)),
    );
    assert_eq!(updated.model.context.delay_ms, 900);
    assert_eq!(updated.model.armed.len(), 1);

    let json = serde_json::to_string(&updated.model).expect("serialize model");
    let restored: StatechartModel<ElmTimerState, ElmTimerCtx> =
        serde_json::from_str(&json).expect("deserialize model");
    assert_eq!(restored.armed, updated.model.armed);
    assert!(ElmTimerDef::machine().can(&restored.machine_state(), &ElmTimerEvent::SetDelay(100),));

    // Firing the originally armed 50ms timer routes through `extract_timer` -> `step_timer`
    // and succeeds even though context.delay_ms changed to 900.
    let done = P::update(
        &restored,
        &ElmTimerMsg::Fired {
            state: ElmTimerState::Waiting,
            ms: 50,
        },
    );
    assert_eq!(done.model.state(), Some(&ElmTimerState::Done));
    assert!(done.model.armed.is_empty());
}

#[test]
fn actor_system_records_dead_letters_for_stopped_or_missing_targets_and_purges_stopped_tasks_d18() {
    let root_machine = DynamicMachine::builder(
        serde_json::from_str(
            r#"{
                "initial": "Ready",
                "states": {
                    "Ready": {
                        "on": {
                            "SEND_MISSING": {"actions": "sendMissing"}
                        }
                    }
                }
            }"#,
        )
        .expect("root schema"),
    )
    .action("sendMissing", |_ctx, _ev, enq| {
        enq.send_to("missing_child", DynamicEvent::new("PING"));
    })
    .build();

    let mut system = ActorSystem::new("root", ActorLogic::from_machine(root_machine));
    system.run().expect("settle");

    // External `send` to an unknown actor still fails loudly.
    assert_eq!(
        system.send("nonexistent", DynamicEvent::new("HI")),
        Err(ActorError::Unknown(ActorId::new("nonexistent")))
    );

    // Internal `send_to` to a missing target records a DeadLetter instead of aborting.
    system
        .send("root", DynamicEvent::new("SEND_MISSING"))
        .expect("queue SEND_MISSING");
    system
        .run()
        .expect("internal missing send is dead-lettered");

    assert_eq!(
        system.dead_letters(),
        &[DeadLetter {
            sender: Some(ActorId::new("root")),
            target: ActorId::new("missing_child"),
            event: DynamicEvent::new("PING"),
        }]
    );
    assert_eq!(system.take_dead_letters().len(), 1);
    assert!(system.dead_letters().is_empty());

    // Subtree stop (D18) purges pending child tasks and dead-letters queued inbox mail.
    let worker_machine = DynamicMachine::from_json_str(
        r#"{"initial":"Active","states":{"Active":{"on":{"WORK":{}}}}}"#,
    )
    .expect("worker");
    system
        .spawn(
            &ActorId::new("root"),
            "worker",
            ActorLogic::from_machine(worker_machine),
        )
        .expect("spawn worker");
    system
        .spawn(
            &ActorId::new("worker"),
            "worker_job",
            ActorLogic::from_effect(|| Effect::succeed(DynamicEvent::new("JOB_DONE"))),
        )
        .expect("spawn child effect");
    system
        .send("worker", DynamicEvent::new("QUEUED_MSG"))
        .expect("queue message before stop");

    assert!(system.stop(&ActorId::new("worker")));
    assert!(!system.exists(&ActorId::new("worker")));
    assert!(!system.exists(&ActorId::new("worker_job")));

    // Uncollected task from stopped grandchild was purged.
    assert!(system.take_tasks().is_empty());

    // Queued inbox mail targeting the stopped subtree moved to dead_letters.
    let dl = system.take_dead_letters();
    assert_eq!(dl.len(), 1);
    assert_eq!(dl[0].target, ActorId::new("worker"));
    assert_eq!(dl[0].event.r#type, "QUEUED_MSG");
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
enum ObsState {
    Start,
    Mid,
    Sub,
    SubInitial,
    SubEnd,
    RaisedDone,
    Timed,
}

#[derive(Debug, Clone)]
enum ObsEv {
    Init,
    Go,
    RaiseFinish,
}

#[test]
fn step_observed_and_microsteps_capture_every_microstep_in_order() {
    let machine: Machine<ObsState, ObsEv, u32, &'static str> = Machine::builder(ObsState::Start, 0)
        .state(ObsState::Start, |n| {
            let go = n
                .transition(ObsState::Mid)
                .on(|e| matches!(e, ObsEv::Go))
                .assign(|ctx, _| *ctx += 1);
            let init = n
                .transition(ObsState::Timed)
                .on(|e| matches!(e, ObsEv::Init));
            n.on(go).on(init)
        })
        .state(ObsState::Mid, |n| {
            let always_tr = n.transition(ObsState::Sub).assign(|ctx, _| *ctx += 10);
            n.always(always_tr)
        })
        .compound(ObsState::Sub, ObsState::SubInitial, |n| {
            let done_tr = n.transition(ObsState::RaisedDone).action(|ctx, _, enq| {
                *ctx += 100;
                enq.exec("completed_sub");
            });
            n.on_done(done_tr)
        })
        .state(ObsState::SubInitial, |n| {
            let finish = n
                .transition(ObsState::SubEnd)
                .on(|e| matches!(e, ObsEv::RaiseFinish));
            n.with_parent(ObsState::Sub)
                .entry(|_, enq| enq.raise(ObsEv::RaiseFinish))
                .on(finish)
        })
        .final_state(ObsState::SubEnd, |n| n.with_parent(ObsState::Sub))
        .state(ObsState::RaisedDone, |n| n)
        .state(ObsState::Timed, |n| {
            let back = n.transition(ObsState::Start);
            n.after(Duration::from_millis(30), back)
        })
        .build();

    let mut init_obs = Vec::new();
    let started = machine.start_observed(&0, &ObsEv::Init, &mut init_obs);
    assert_eq!(init_obs.len(), 1);
    assert_eq!(init_obs[0].kind, MicrostepKind::Init);
    assert_eq!(init_obs[0].entered, vec![ObsState::Start]);

    let s0 = MachineState {
        configuration: started.configuration,
        history: started.history,
        context: started.context,
        armed: started.armed,
    };

    let (outcome, trace) = machine.microsteps(&s0, &ObsEv::Go);
    assert_eq!(outcome.current_state, ObsState::RaisedDone);
    assert_eq!(outcome.context, 111);
    assert_eq!(outcome.commands, vec!["completed_sub"]);

    let kinds: Vec<MicrostepKind> = trace.iter().map(|r| r.kind).collect();
    assert_eq!(
        kinds,
        vec![
            MicrostepKind::Event,
            MicrostepKind::Always,
            MicrostepKind::Raised,
            MicrostepKind::Done,
        ]
    );
    assert_eq!(trace[0].exited, vec![ObsState::Start]);
    assert_eq!(trace[0].entered, vec![ObsState::Mid]);
    assert_eq!(trace[1].entered, vec![ObsState::Sub, ObsState::SubInitial]);
    assert_eq!(trace[2].entered, vec![ObsState::SubEnd]);
    assert_eq!(trace[3].entered, vec![ObsState::RaisedDone]);
    assert_eq!(trace[3].commands, vec!["completed_sub"]);

    let mut step_obs = Vec::new();
    let direct = machine.step_observed(&s0, &ObsEv::Go, &mut step_obs);
    assert_eq!(direct.current_state, ObsState::RaisedDone);
    assert_eq!(step_obs.len(), 4);

    // Timer firing is observed with `MicrostepKind::Timer`.
    let timed_out = machine.step_state(&s0, &ObsEv::Init);
    let timed_state = MachineState {
        configuration: timed_out.configuration,
        history: timed_out.history,
        context: timed_out.context,
        armed: timed_out.armed,
    };
    let mut timer_obs = Vec::new();
    let timer_res = machine.step_timer_observed(
        &timed_state,
        &TimerFired::new(ObsState::Timed, Duration::from_millis(30)),
        &ObsEv::Init,
        &mut timer_obs,
    );
    assert_eq!(timer_res.current_state, ObsState::Start);
    assert_eq!(timer_obs.len(), 1);
    assert_eq!(timer_obs[0].kind, MicrostepKind::Timer);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum JsonState {
    Flow,
    Hist,
    Step1,
    Step2,
    StepFinal,
    Branch,
    Par,
    RegA,
    A1,
    RegB,
    B1,
}

#[derive(Debug, Clone)]
enum JsonEv {
    Next,
    Reenter,
}

#[test]
fn to_machine_json_exports_hierarchical_xstate_v6_schema() {
    let machine: Machine<JsonState, JsonEv, (), ()> = Machine::builder(JsonState::Flow, ())
        .compound(JsonState::Flow, JsonState::Step1, |n| {
            let done_tr = n.transition(JsonState::Par);
            n.on_done(done_tr)
        })
        .history(JsonState::Hist, JsonState::Step1, |n| {
            n.with_parent(JsonState::Flow)
        })
        .state(JsonState::Step1, |n| {
            let next_tr = n
                .transition(JsonState::Branch)
                .on_key("NEXT", |e| matches!(e, JsonEv::Next));
            let after_tr = n.transition(JsonState::StepFinal);
            n.with_parent(JsonState::Flow)
                .on(next_tr)
                .after(Duration::from_millis(200), after_tr)
        })
        .choice(JsonState::Branch, |n| {
            let tr = n.transition(JsonState::Step2);
            n.with_parent(JsonState::Flow).always(tr)
        })
        .state(JsonState::Step2, |n| {
            let reenter_tr = n
                .transition(JsonState::Step2)
                .reenter()
                .on_key("REENTER", |e| matches!(e, JsonEv::Reenter));
            n.with_parent(JsonState::Flow).on(reenter_tr)
        })
        .final_state(JsonState::StepFinal, |n| n.with_parent(JsonState::Flow))
        .parallel(JsonState::Par, |n| n)
        .compound(JsonState::RegA, JsonState::A1, |n| {
            n.with_parent(JsonState::Par)
        })
        .state(JsonState::A1, |n| n.with_parent(JsonState::RegA))
        .compound(JsonState::RegB, JsonState::B1, |n| {
            n.with_parent(JsonState::Par)
        })
        .state(JsonState::B1, |n| n.with_parent(JsonState::RegB))
        .build();

    let json = machine.to_machine_json();
    assert_eq!(json["initial"], "Flow");
    let step2 = machine.step(&JsonState::Step1, &(), &JsonEv::Next);
    assert_eq!(step2.current_state, JsonState::Step2);
    assert_eq!(
        machine
            .step(&JsonState::Step2, &(), &JsonEv::Reenter)
            .current_state,
        JsonState::Step2
    );

    let flow = &json["states"]["Flow"];
    assert_eq!(flow["initial"], "Step1");
    assert_eq!(flow["onDone"]["target"], "Par");
    assert_eq!(flow["states"]["Hist"]["type"], "history");
    assert_eq!(flow["states"]["Hist"]["history"], "shallow");
    assert_eq!(flow["states"]["Hist"]["target"], "Step1");
    assert_eq!(flow["states"]["Branch"]["type"], "choice");
    assert_eq!(flow["states"]["Branch"]["always"][0]["target"], "Step2");
    assert_eq!(flow["states"]["Step1"]["on"]["NEXT"]["target"], "Branch");
    assert_eq!(
        flow["states"]["Step1"]["after"]["200"]["target"],
        "StepFinal"
    );
    assert_eq!(flow["states"]["Step2"]["on"]["REENTER"]["reenter"], true);
    assert_eq!(flow["states"]["StepFinal"]["type"], "final");
    assert_eq!(json["states"]["Par"]["type"], "parallel");

    // DynamicMachine exports XState v6 JSON that round-trips back through DynamicMachine.
    let dyn_src = r#"{
        "initial": "Wait",
        "states": {
            "Wait": {"on": {"DECIDE": "Fork"}},
            "Fork": {
                "type": "choice",
                "always": [{"target": "Done"}]
            },
            "Done": {"type": "final"}
        }
    }"#;
    let dyn_machine = DynamicMachine::from_json_str(dyn_src).expect("parse src");
    let exported = dyn_machine.to_machine_json();
    let cfg: MachineConfig = serde_json::from_value(exported).expect("MachineConfig round-trip");
    let rebuilt = DynamicMachine::builder(cfg).build();
    let out = rebuilt.step(
        rebuilt.initial_state(),
        rebuilt.initial_context(),
        &DynamicEvent::new("DECIDE"),
    );
    assert_eq!(out.current_state, "Done");
    assert!(out.done);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum MacroState {
    Flow,
    FlowHist,
    FlowDeepHist,
    Idle,
    Check,
    High,
    Low,
    Timed,
    Timeout,
    InnerDone,
    ParPhase,
    RegA,
    A1,
    A2,
    RegB,
    B1,
    B2,
    Complete,
}

#[derive(Debug, Clone)]
enum MacroEv {
    Add(u32),
    Ping(String),
    Leave,
    ReenterHist,
    StartTimer,
    FinishFlow,
    DoneA,
    DoneB,
    Tick,
}

#[derive(Debug, Clone, Default)]
struct MacroCtx {
    sum: u32,
    limit: u32,
    pings: Vec<String>,
    exited_idle: bool,
}

#[test]
fn statechart_macro_builds_labeled_chart_with_payload_destructuring() {
    let machine = statechart! {
        state: MacroState,
        event: MacroEv,
        context: MacroCtx,
        command: String,
        initial: MacroState::Flow,
        context_init: MacroCtx { sum: 0, limit: 15, pings: Vec::new(), exited_idle: false },
        states: {
            compound MacroState::Flow (initial: MacroState::Idle) {
                on: {
                    MacroEv::FinishFlow => target MacroState::ParPhase,
                },
                on_done: target MacroState::ParPhase,
                states: {
                    history MacroState::FlowHist (fallback: MacroState::Idle) {},
                    deep_history MacroState::FlowDeepHist (fallback: MacroState::Idle) {},
                    state MacroState::Idle {
                        exit: |ctx, _| ctx.exited_idle = true,
                        on: {
                            MacroEv::Add(n) if *n > 0 => {
                                target: MacroState::Check,
                                guard: |ctx| ctx.sum + *n <= ctx.limit,
                                action: |ctx, enq| {
                                    ctx.sum += *n;
                                    enq.exec(format!("added:{}", *n));
                                },
                                label: "add_payload",
                            },
                            MacroEv::Ping(msg) => {
                                action: |ctx, _| ctx.pings.push(msg.clone()),
                                label: "record_ping",
                            },
                            MacroEv::StartTimer => target MacroState::Timed,
                        },
                    },
                    choice MacroState::Check {
                        always: [
                            { target: MacroState::High, guard: |ctx| ctx.sum >= 10 },
                            target MacroState::Low,
                        ],
                    },
                    state MacroState::High {
                        on: {
                            MacroEv::Leave => target MacroState::Low,
                            MacroEv::FinishFlow => target MacroState::InnerDone,
                        },
                    },
                    state MacroState::Low {
                        on: {
                            MacroEv::ReenterHist => target MacroState::FlowHist,
                            MacroEv::FinishFlow => target MacroState::InnerDone,
                        },
                    },
                    state MacroState::Timed {
                        after: {
                            Duration::from_millis(25) => target MacroState::Timeout,
                        },
                    },
                    state MacroState::Timeout {},
                    final MacroState::InnerDone {},
                },
            },
            parallel MacroState::ParPhase {
                on_done: target MacroState::Complete,
                states: {
                    compound MacroState::RegA (initial: MacroState::A1) {
                        states: {
                            state MacroState::A1 {
                                on: { MacroEv::DoneA => target MacroState::A2 },
                            },
                            final MacroState::A2 {},
                        },
                    },
                    compound MacroState::RegB (initial: MacroState::B1) {
                        states: {
                            state MacroState::B1 {
                                on: { MacroEv::DoneB => target MacroState::B2 },
                            },
                            final MacroState::B2 {},
                        },
                    },
                },
            },
            final MacroState::Complete {},
        },
    };

    // Guard blocks payload > limit.
    let too_big = machine.step(
        &MacroState::Idle,
        &MacroCtx {
            limit: 15,
            ..Default::default()
        },
        &MacroEv::Add(20),
    );
    assert!(!too_big.transitioned);
    assert_eq!(too_big.current_state, MacroState::Idle);

    // Targetless payload action keeps state and records ping.
    let pinged = machine.step(
        &MacroState::Idle,
        &MacroCtx {
            limit: 15,
            ..Default::default()
        },
        &MacroEv::Ping("hello".into()),
    );
    assert_eq!(pinged.current_state, MacroState::Idle);
    assert_eq!(pinged.context.pings, vec!["hello".to_string()]);
    assert!(!pinged.context.exited_idle);

    // Payload binding + guard + action + choice pseudo-state branching.
    let added = machine.step(
        &MacroState::Idle,
        &MacroCtx {
            limit: 15,
            ..Default::default()
        },
        &MacroEv::Add(12),
    );
    assert_eq!(added.current_state, MacroState::High);
    assert_eq!(added.context.sum, 12);
    assert!(added.context.exited_idle);
    assert_eq!(added.commands, vec!["added:12".to_string()]);

    let left = machine.step_state(
        &MachineState {
            configuration: added.configuration,
            history: added.history,
            context: added.context,
            armed: added.armed,
        },
        &MacroEv::Leave,
    );
    assert_eq!(left.current_state, MacroState::Low);
    let hist_back = machine.step_state(
        &MachineState {
            configuration: left.configuration,
            history: left.history,
            context: left.context,
            armed: left.armed,
        },
        &MacroEv::ReenterHist,
    );
    assert_eq!(hist_back.current_state, MacroState::Idle);

    // `after` timer transitions cleanly.
    let armed = machine.step(
        &MacroState::Idle,
        &MacroCtx::default(),
        &MacroEv::StartTimer,
    );
    assert_eq!(armed.current_state, MacroState::Timed);
    let timed = machine.step_timer(
        &MachineState {
            configuration: armed.configuration,
            history: armed.history,
            context: armed.context,
            armed: armed.armed,
        },
        &TimerFired::new(MacroState::Timed, Duration::from_millis(25)),
        &MacroEv::Tick,
    );
    assert_eq!(timed.current_state, MacroState::Timeout);

    // Parallel completion moves `ParPhase -> Complete`.
    let par = machine.step(
        &MacroState::Idle,
        &MacroCtx::default(),
        &MacroEv::FinishFlow,
    );
    assert_eq!(
        par.configuration.leaves().copied().collect::<Vec<_>>(),
        vec![MacroState::A1, MacroState::B1]
    );
    let half = machine.step_state(
        &MachineState {
            configuration: par.configuration,
            history: par.history,
            context: par.context,
            armed: par.armed,
        },
        &MacroEv::DoneA,
    );
    assert_eq!(
        half.configuration.leaves().copied().collect::<Vec<_>>(),
        vec![MacroState::A2, MacroState::B1]
    );
    let done = machine.step_state(
        &MachineState {
            configuration: half.configuration,
            history: half.history,
            context: half.context,
            armed: half.armed,
        },
        &MacroEv::DoneB,
    );
    assert_eq!(done.current_state, MacroState::Complete);
    assert!(done.done);

    // Mermaid & MachineJSON both reflect the macro-built chart.
    let mermaid = machine.to_mermaid();
    assert!(mermaid.contains("add_payload"));
    let json = machine.to_machine_json();
    assert_eq!(json["states"]["Flow"]["initial"], "Idle");
    assert_eq!(json["states"]["ParPhase"]["type"], "parallel");
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum TypedAfterState {
    Outer,
    Waiting,
    TimedOut,
    ParentHandled,
}

#[derive(Debug, Clone)]
enum TypedAfterEv {
    Init,
    Other,
}

#[test]
fn typed_after_transitions_do_not_match_unhandled_events_and_bubble_to_parent() {
    let machine: Machine<TypedAfterState, TypedAfterEv, (), ()> =
        Machine::builder(TypedAfterState::Outer, ())
            .compound(TypedAfterState::Outer, TypedAfterState::Waiting, |n| {
                let tr = n
                    .transition(TypedAfterState::ParentHandled)
                    .on(|e| matches!(e, TypedAfterEv::Other));
                n.on(tr)
            })
            .state(TypedAfterState::Waiting, |n| {
                let to_timeout = n.transition(TypedAfterState::TimedOut);
                n.with_parent(TypedAfterState::Outer)
                    .after(Duration::from_millis(100), to_timeout)
            })
            .final_state(TypedAfterState::TimedOut, |n| n)
            .final_state(TypedAfterState::ParentHandled, |n| n)
            .build();

    let waiting = MachineState::at(TypedAfterState::Waiting, ());
    assert!(!machine.can(&waiting, &TypedAfterEv::Init));
    assert!(machine.can(&waiting, &TypedAfterEv::Other));

    let unchanged = machine.step_state(&waiting, &TypedAfterEv::Init);
    assert!(!unchanged.transitioned);
    assert_eq!(unchanged.current_state, TypedAfterState::Waiting);

    let bubbled = machine.step_state(&waiting, &TypedAfterEv::Other);
    assert_eq!(bubbled.current_state, TypedAfterState::ParentHandled);
}

#[test]
fn multi_branch_after_transitions_on_same_delay_evaluate_guards_in_order_and_export_array() {
    let machine: Machine<ChoiceState, ChoiceEv, i32, ()> = Machine::builder(ChoiceState::Start, 0)
        .state(ChoiceState::Start, |n| {
            let first = n.transition(ChoiceState::High).guard(|ctx, _| *ctx >= 10);
            let second = n.transition(ChoiceState::Low);
            n.after(Duration::from_millis(50), first)
                .after(Duration::from_millis(50), second)
        })
        .final_state(ChoiceState::High, |n| n)
        .final_state(ChoiceState::Low, |n| n)
        .build();

    let entered = machine.enter(&3);
    assert_eq!(
        entered.timers,
        vec![TimerIntent::Start {
            state: ChoiceState::Start,
            duration: Duration::from_millis(50),
        }]
    );

    let fired_low = machine.step_timer(
        &MachineState {
            configuration: entered.configuration.clone(),
            history: entered.history.clone(),
            context: 3,
            armed: entered.armed.clone(),
        },
        &TimerFired::new(ChoiceState::Start, Duration::from_millis(50)),
        &ChoiceEv::Eval(3),
    );
    assert_eq!(fired_low.current_state, ChoiceState::Low);

    let fired_high = machine.step_timer(
        &MachineState {
            configuration: entered.configuration,
            history: entered.history,
            context: 15,
            armed: entered.armed,
        },
        &TimerFired::new(ChoiceState::Start, Duration::from_millis(50)),
        &ChoiceEv::Eval(15),
    );
    assert_eq!(fired_high.current_state, ChoiceState::High);

    let json = machine.to_machine_json();
    let branches = json["states"]["Start"]["after"]["50"]
        .as_array()
        .expect("multi-branch after serializes as array");
    assert_eq!(branches.len(), 2);
    assert_eq!(branches[0]["target"], "High");
    assert_eq!(branches[1]["target"], "Low");
}

#[test]
fn actor_system_stop_nets_uncollected_starts_and_cancels_armed_timers() {
    let timed_child = DynamicMachine::from_json_str(
        r#"{
            "initial": "Wait",
            "states": {
                "Wait": {"after": {"120": "Done"}},
                "Done": {"type": "final"}
            }
        }"#,
    )
    .expect("timed child");
    let root = DynamicMachine::from_json_str(r#"{"initial": "Idle", "states": {"Idle": {}}}"#)
        .expect("root");

    // Case 1: uncollected Start is removed on stop before host calls take_timers().
    let mut system1 = ActorSystem::new("root", ActorLogic::from_machine(root.clone()));
    system1
        .spawn(
            &ActorId::new("root"),
            "child",
            ActorLogic::from_machine(timed_child.clone()),
        )
        .expect("spawn child");
    assert!(system1.stop(&ActorId::new("child")));
    assert!(system1.take_timers().is_empty());

    // Case 2: already-collected timer gets a Cancel intent when the child stops later.
    let mut system2 = ActorSystem::new("root", ActorLogic::from_machine(root));
    system2
        .spawn(
            &ActorId::new("root"),
            "child",
            ActorLogic::from_machine(timed_child),
        )
        .expect("spawn child");
    assert_eq!(
        system2.take_timers(),
        vec![(
            ActorId::new("child"),
            TimerIntent::Start {
                state: "Wait".to_string(),
                duration: Duration::from_millis(120),
            },
        )]
    );
    assert!(system2.stop(&ActorId::new("child")));
    assert_eq!(
        system2.take_timers(),
        vec![(
            ActorId::new("child"),
            TimerIntent::Cancel {
                state: "Wait".to_string(),
                duration: Duration::from_millis(120),
            },
        )]
    );
}

#[test]
fn statechart_macro_accepts_colon_shorthand_in_always_and_on_arms() {
    let machine = statechart! {
        state: ChoiceState,
        event: ChoiceEv,
        context: i32,
        command: (),
        initial: ChoiceState::Decide,
        states: {
            state ChoiceState::Start {
                on: {
                    ChoiceEv::Eval(_) => target: ChoiceState::Decide,
                },
            },
            choice ChoiceState::Decide {
                always: [
                    { target: ChoiceState::High, guard: |ctx| *ctx >= 10 },
                    target: ChoiceState::Low,
                ],
            },
            final ChoiceState::High {},
            final ChoiceState::Low {},
        },
    };

    let low = machine.start(&2, &ChoiceEv::Eval(0));
    assert_eq!(low.current_state, ChoiceState::Low);
    let high = machine.start(&12, &ChoiceEv::Eval(0));
    assert_eq!(high.current_state, ChoiceState::High);
}
