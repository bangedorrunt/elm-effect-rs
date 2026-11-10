# Statecharts and Harel Actor Lifecycles in `elm-effect-rs`

This reference guides AI agents and developers implementing statecharts in `elm-effect-rs` using the `elm-effect-xstate` crate ([ADR-0007](file:///Users/bangedorrunt/workspace/elm-effect-rs/docs/decisions/0007-port-xstate-v6-into-rust-as-elm-effect-xstate.md), which supersedes ADR-0006).

---

## 1. Architectural Placement & Carrier Law

`elm-effect-xstate` provides first-class Harel statecharts and XState v6 behavioral parity for `elm-effect-rs`.

### The Cardinal Carrier Rule
**Never spawn a secondary background scheduler or runtime thread for statecharts.**

- A statechart is a purely functional transition machine.
- All state transitions, guards, context updates, and eventless micro-steps compile directly into a deterministic Elm loop reducer: [`elm_effect_elm::Program::update`](file:///Users/bangedorrunt/workspace/elm-effect-rs/crates/elm-effect-elm/src/lib.rs).
- Side-effect commands returned from transitions are executed at the process edge through [`elm_effect_elm::Interpret`](file:///Users/bangedorrunt/workspace/elm-effect-rs/crates/elm-effect-elm/src/lib.rs).

---

## 2. Core Statechart Laws

Statechart evaluation in `elm-effect-xstate` strictly follows the five fundamental laws:

1. **Law L1 (Strict Topological Execution Order)**:
   For any transition from active leaf $S$ to target $T$ with Least Common Ancestor $\text{LCA}(S, T)$:
   $$\text{Exit}(S \dots \text{LCA}) \longrightarrow \text{Transition Actions} \longrightarrow \text{Context Assignments} \longrightarrow \text{Entry}(\text{LCA} \dots T)$$
   In a self-transition ($S = T$), $S$ exits and re-enters.
2. **Law L2 (Eventless Micro-step Quiescence)**:
   Transitions defined under `always` evaluate immediately upon state entry without waiting for an external event. They cascade sequentially in micro-steps until reaching a stable configuration (quiescence) or an iteration bound (100 steps) preventing infinite loops.
3. **Law L3 (Hierarchical Event Bubbling)**:
   When an active child state receives an event with no matching transition, the event bubbles up the ancestry path ($S \to \text{parent} \to \text{grandparent}$) until an ancestor handles it.
4. **Law L4 (Deterministic Guard Priority)**:
   Transitions for a given event are evaluated in declaration order. The first transition whose guard evaluates to `true` is taken; subsequent branches are ignored.
5. **Law L5 (Unified Action Enqueue)**:
   Handlers receive an [`ActionQueue`](file:///Users/bangedorrunt/workspace/elm-effect-rs/crates/elm-effect-xstate/src/action.rs#L5-L52) (`enq`), following XState v6's action discipline:
   - `enq.exec(cmd)`: Emits a managed command to the host runtime.
   - `enq.raise(event)`: Emits an internal event to re-enter the machine fold.
   - `enq.emit(payload)`: Emits telemetry/inspection data.

---

## 3. When to Use Statecharts vs Pure Elm Program

| Scenario | Recommend | Reason |
| --- | --- | --- |
| Hierarchical states (e.g. `Active` with `Idle`/`Running`) | **`elm-effect-xstate`** | Parent handles `Stop`/`Reset` via bubbling; no manual enum flattening. |
| Eventless cascading logic (e.g. validation cascades) | **`elm-effect-xstate`** | `always` transitions evaluate in micro-steps to quiescence automatically. |
| Linear data processing or plain form input | **`elm-effect-elm`** | Single model, no nested states or lifecycle action requirements. |
| Workflow requiring durable pause/resume | **`elm-effect-durable`** + **`elm-effect-xstate`** | Durable engine manages run leases; statechart manages discrete local state. |

---

## 4. Canonical Scenarios & Examples

### Scenario 1: Traffic Light (Atomic Cycles with Context Mutation)

A sequential cycle demonstrating atomic states, event matching, and context mutation (`assign`):

```rust
use elm_effect_xstate::prelude::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum LightState {
    Green,
    Yellow,
    Red,
}

#[derive(Debug, Clone)]
enum LightEvent {
    Timer,
}

#[derive(Debug, Clone, Default)]
struct LightContext {
    cycles: u32,
}

let machine: Machine<LightState, LightEvent, LightContext, ()> =
    Machine::builder(LightState::Green, LightContext::default())
        .state(LightState::Green, |node| {
            let to_yellow = node
                .transition(LightState::Yellow)
                .on(|e| matches!(e, LightEvent::Timer));
            node.on(to_yellow)
        })
        .state(LightState::Yellow, |node| {
            let to_red = node
                .transition(LightState::Red)
                .on(|e| matches!(e, LightEvent::Timer));
            node.on(to_red)
        })
        .state(LightState::Red, |node| {
            let to_green = node
                .transition(LightState::Green)
                .on(|e| matches!(e, LightEvent::Timer))
                .assign(|ctx, _| ctx.cycles += 1);
            node.on(to_green)
        })
        .build();

let outcome = machine.step(&LightState::Green, &LightContext::default(), &LightEvent::Timer);
assert_eq!(outcome.current_state, LightState::Yellow);
```

---

### Scenario 2: Media Player (Hierarchical Compound States with Event Bubbling)

Demonstrating hierarchical ancestry, initial child resolution, and parent-level event bubbling:

```rust
use elm_effect_xstate::prelude::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum PlayerState {
    Active,
    Idle,
    Playing,
    Paused,
    Stopped,
}

#[derive(Debug, Clone)]
enum PlayerEvent {
    Play,
    Pause,
    Stop,
}

#[derive(Debug, Clone, Default)]
struct PlayerContext {
    track: Option<String>,
}

let machine: Machine<PlayerState, PlayerEvent, PlayerContext, ()> =
    Machine::builder(PlayerState::Active, PlayerContext::default())
        // Compound parent state with initial child Idle
        .compound(PlayerState::Active, PlayerState::Idle, |node| {
            // Parent-level handler: handles Stop from any active substate!
            let to_stopped = node
                .transition(PlayerState::Stopped)
                .on(|e| matches!(e, PlayerEvent::Stop));
            node.on(to_stopped)
        })
        .state(PlayerState::Idle, |node| {
            let to_playing = node
                .transition(PlayerState::Playing)
                .on(|e| matches!(e, PlayerEvent::Play));
            node.with_parent(PlayerState::Active).on(to_playing)
        })
        .state(PlayerState::Playing, |node| {
            let to_paused = node
                .transition(PlayerState::Paused)
                .on(|e| matches!(e, PlayerEvent::Pause));
            node.with_parent(PlayerState::Active).on(to_paused)
        })
        .state(PlayerState::Paused, |node| {
            let to_playing = node
                .transition(PlayerState::Playing)
                .on(|e| matches!(e, PlayerEvent::Play));
            node.with_parent(PlayerState::Active).on(to_playing)
        })
        .final_state(PlayerState::Stopped, |node| node)
        .build();

// 1. Initial resolution: resolving Active resolves to leaf Idle
assert_eq!(machine.resolve_leaf(&PlayerState::Active), PlayerState::Idle);

// 2. Sibling transition within compound state: Idle -> Playing (Active does not exit)
let outcome1 = machine.step(&PlayerState::Idle, &PlayerContext::default(), &PlayerEvent::Play);
assert_eq!(outcome1.current_state, PlayerState::Playing);

// 3. Bubbling: Playing has no Stop handler, so parent Active handles it, transitioning to Stopped
let outcome2 = machine.step(&PlayerState::Playing, &PlayerContext::default(), &PlayerEvent::Stop);
assert_eq!(outcome2.current_state, PlayerState::Stopped);
```

---

### Scenario 3: Turnstile / Payment Gate (Guards & Managed Effect Commands)

Demonstrating conditional guards, context assignments, and runtime command emission (`exec`):

```rust
use elm_effect_xstate::prelude::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum GateState {
    Locked,
    Unlocked,
}

#[derive(Debug, Clone)]
enum GateEvent {
    Coin { amount_cents: u32 },
    Push,
}

#[derive(Debug, Clone, Default)]
struct GateContext {
    balance_cents: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum GateCmd {
    DisengageSolenoid,
    EngageSolenoid,
}

let machine: Machine<GateState, GateEvent, GateContext, GateCmd> =
    Machine::builder(GateState::Locked, GateContext::default())
        .state(GateState::Locked, |node| {
            let to_unlocked = node
                .transition(GateState::Unlocked)
                .on(|e| matches!(e, GateEvent::Coin { .. }))
                .guard(|ctx, e| match e {
                    GateEvent::Coin { amount_cents } => ctx.balance_cents + amount_cents >= 50,
                    _ => false,
                })
                .assign(|ctx, e| {
                    if let GateEvent::Coin { amount_cents } = e {
                        ctx.balance_cents += amount_cents;
                    }
                })
                .exec(GateCmd::DisengageSolenoid);

            node.on(to_unlocked)
        })
        .state(GateState::Unlocked, |node| {
            let to_locked = node
                .transition(GateState::Locked)
                .on(|e| matches!(e, GateEvent::Push))
                .exec(GateCmd::EngageSolenoid);

            node.on(to_locked)
        })
        .build();
```

---

### Scenario 4: Eventless Transient Loops (`always` Cascading to Quiescence)

Demonstrating eventless microsteps cascading sequentially until conditions settle:

```rust
use elm_effect_xstate::prelude::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum LoopState {
    Start,
    Counting,
    Done,
}

#[derive(Debug, Clone)]
enum LoopEvent {
    Run,
}

#[derive(Debug, Clone, Default)]
struct LoopContext {
    count: u32,
}

let machine: Machine<LoopState, LoopEvent, LoopContext, ()> =
    Machine::builder(LoopState::Start, LoopContext::default())
        .state(LoopState::Start, |node| {
            let to_counting = node
                .transition(LoopState::Counting)
                .on(|e| matches!(e, LoopEvent::Run));
            node.on(to_counting)
        })
        .state(LoopState::Counting, |node| {
            let loop_step = node
                .transition(LoopState::Counting)
                .guard(|ctx, _| ctx.count < 3)
                .assign(|ctx, _| ctx.count += 1);

            let to_done = node
                .transition(LoopState::Done)
                .guard(|ctx, _| ctx.count >= 3);

            node.always(loop_step).always(to_done)
        })
        .final_state(LoopState::Done, |node| node)
        .build();

// In one single event step, the engine evaluates microsteps until count == 3 and settles on Done
let outcome = machine.step(&LoopState::Start, &LoopContext::default(), &LoopEvent::Run);
assert_eq!(outcome.current_state, LoopState::Done);
assert_eq!(outcome.context.count, 3);
```

---

### Scenario 5: Schema-Driven Dynamic Statecharts & Conformance (`DynamicMachine`)

Loading statecharts from declarative JSON schemas exported from Stately Studio or CMS:

```rust
use elm_effect_xstate::prelude::*;

let json_schema = r#"{
    "initial": "Idle",
    "context": { "score": 85 },
    "states": {
        "Idle": {
            "on": {
                "GRADE": [
                    { "target": "Passed", "guard": "score >= 70", "actions": ["award_diploma"] },
                    { "target": "Failed", "actions": ["send_remedial"] }
                ]
            }
        },
        "Passed": { "type": "final" },
        "Failed": { "type": "final" }
    }
}"#;

let dynamic_machine = DynamicMachine::from_json_str(json_schema).expect("valid schema");
let outcome = dynamic_machine.step("Idle", dynamic_machine.initial_context(), &DynamicEvent::new("GRADE"));

assert_eq!(outcome.current_state, "Passed");
assert_eq!(outcome.context.log, vec!["award_diploma"]);
```

---

## 5. Integrating with the Elm Loop (`Program`)

To drive a statechart inside an application loop:

1. Define a marker struct implementing [`StatechartDefinition`](file:///Users/bangedorrunt/workspace/elm-effect-rs/crates/elm-effect-xstate/src/adapter.rs#L14-L37).
2. Wrap it with [`StatechartProgram`](file:///Users/bangedorrunt/workspace/elm-effect-rs/crates/elm-effect-xstate/src/adapter.rs#L39-L74).
3. Boot directly in a [`elm_effect_elm::Loop`](file:///Users/bangedorrunt/workspace/elm-effect-rs/crates/elm-effect-elm/src/lib.rs):

```rust
use elm_effect_elm::{Boot, Cmd, Failure, Program};
use elm_effect_xstate::prelude::*;

struct MyStatechartDef;

impl StatechartDefinition for MyStatechartDef {
    type State = PlayerState;
    type Event = PlayerEvent;
    type Context = PlayerContext;
    type Cmd = AppCmd;
    type Msg = AppMsg;
    type Err = String;

    fn machine() -> Machine<Self::State, Self::Event, Self::Context, Self::Cmd> {
        // Construct machine here
    }

    fn extract_event(msg: &Self::Msg) -> Option<Self::Event> {
        match msg {
            AppMsg::UserAction(e) => Some(e.clone()),
            _ => None,
        }
    }

    fn failure(_failure: &Failure<Self::Err>) -> Self::Msg {
        AppMsg::Failed
    }
}

type MyProgram = StatechartProgram<MyStatechartDef>;
```

---

## 6. Parallel regions, history, delays, completion

- **Parallel**: `.parallel(id, ...)`; children with `parent = id` are regions. Entering activates one leaf per region. One event is offered to every region, and a deeper selection preempts an ancestor selection.
- **History**: `.history(id, fallback, ...)` and `.deep_history(id, fallback, ...)`; the parent must be a compound state. History is recorded when that parent exits. A history node with no record enters its fallback.
- **Delays**: `node.after(Duration, transition)` arms a `TimerIntent::Start` on entry and a `TimerIntent::Cancel` on exit. Nothing reads a clock. Call `machine.step_timer(&state, &TimerFired::new(id, duration), &event)`; a fire for an exited state is a no-op.
- **Completion**: a final child completes its compound parent; a parallel node completes when every region does. `node.on_done(transition)` takes the completion edge. `outcome.done` reports a completed root, including a top-level final sibling.
- **Actors**: `ActorSystem` runs machine and reducer actors in-process with no extra runtime. Actions reach it through `enq.spawn`, `enq.send_to`, and `enq.stop`; `run` applies directives and is bounded (`ActorError::StepBudget`). Each actor owns a `Scope` forked from its parent's, so `stop` closes descendants deepest-first and runs finalizers LIFO. A child that reaches a top-level final sends `xstate.done.actor.<id>` to its parent.
- **Actor logic**: `from_machine`, `from_transition`, `from_effect`, `from_promise`, `from_stream`, and `from_callback`. Effect and promise actors emit `ActorTask` values for the host; a stream forwards values to its parent; a callback receives a `CallbackSender` at spawn. Run host tasks inside `system.scope(&task.actor)` so `stop` interrupts the fiber.
- **Run state**: `Machine::enter` starts; `Machine::step_state` takes a `MachineState` (configuration, history, context). `Machine::step(&leaf, &ctx, &event)` is the one-region short form.
- **Export**: `machine.to_mermaid()`, topology-only `machine.to_tla()` (model-checked by `scripts/verify-formal.sh` via `examples/emit_tla.rs`), and `TransitionRecord::from_outcome(...).append(&log, stamp)`. The stamp is the host's `(producer_id, epoch, seq)`; this crate never mints one. `TransitionRecord::replay` rebuilds a configuration from a continuous chain without running actions. `TransitionRecord::with_snapshot` plus `ActorSystem::resume` restore a machine actor without re-running the active states' entry actions.

## 7. Testing & Conformance Verification

- **Pure Step Testing**: Test statecharts synchronously using `machine.step(...)` or `machine.step_state(...)`. No mock threads, sockets, or clocks required.
- **Delayed Transitions**: The engine returns `TimerIntent` values. A host that runs on [`elm-effect-test::TestClock`](file:///Users/bangedorrunt/workspace/elm-effect-rs/crates/elm-effect-test/src/clock.rs) schedules them as commands, so virtual time still needs no wall-clock sleep.
- **Conformance Matrix**: `crates/elm-effect-xstate/tests/conformance/fixtures/` holds 53 declarative vectors across actions, always, after, parallel, history, hierarchy, guards, final states, and actors, including ten ported from XState 6.0.0-beta.1 (commit d56483f) with a `source` field. `conformance_actors.rs` drives actor scenarios through `spawn:`, `send:`, and `stop:` actions; the machine runner supports `raise:` and `emit:`. `conformance_test.rs` enforces a feature matrix naming the proving cases. Run `scripts/sync-xstate-conformance.sh`. The optional `scripts/xstate-differential` Node check compares state outcomes for 41 vectors; it is not the full ~400-case upstream suite.
- **Formal Verification (TLA+ / TLC)**: Four dedicated statechart models in `skills/verify-framework-flows/` are model-checked via `scripts/verify-formal.sh` and documented in `docs/research/state-machines-and-flows.md`:
  - Flow 6: `flow6_statechart_microstep.tla` (Quiescence, LCA resolution, Law L1 ordering)
  - Flow 7: `flow7_statechart_parallel.tla` (Orthogonal isolation, synchronized `onDone` join)
  - Flow 8: `flow8_statechart_actor.tla` (Recursive scope closure, zero child leak on parent exit)
  - Flow 9: `flow9_statechart_timer.tla` (Virtual dwell, preemptive timer invalidation)
