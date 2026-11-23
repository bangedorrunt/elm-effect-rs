---
status: accepted
date: 2026-11-19
decision-makers: operator of the flywheel, toron, beads, and chiebukuro stack
consulted: ADR-0002, ADR-0005, ADR-0007, Stately XState v6 (6.0.0-beta.3), rockorager/plusthree architecture and DEVIATIONS.md
informed: Rust programmers and agents that use elm-effect
---

# Harden `elm-effect-xstate` with Pre-Order Compiled Topology, Generation-Token Timers, Microstep Observers, Rich XState v6 Primitives, and Declarative Macro DX

## Context and Problem Statement

[ADR-0007](0007-port-xstate-v6-into-rust-as-elm-effect-xstate.md) delivered `crates/elm-effect-xstate` with hierarchical compound states, orthogonal parallel regions, shallow/deep history, `always` microsteps, `after` virtualized delays, `ActorSystem` scoped supervision, and `DurableLog` transition journaling.

A deep comparative architectural review against [`rockorager/plusthree`](https://tangled.org/rockorager.dev/plusthree) (an independent Rust XState `6.0.0-beta.3` engine checked against 406 microstep traces and 21 documented XState deviations) surfaced concrete performance bottlenecks, subtle semantic bugs, and missing XState v6 ergonomics in `elm-effect-xstate`:

1. **Runtime Tree-Walk Overhead**:
   `Machine::step_state` walked parent pointers (`ancestry`, `path_from_root`), scanned `self.nodes.values()` linearly inside `children()`, and allocated path vectors inside `ordered.sort_by_key` on every step rather than pre-compiling the state hierarchy once at `MachineBuilder::build()` time.
2. **Timer & Elm Adapter Defects**:
   - [`StatechartModel::machine_state`](../../crates/elm-effect-xstate/src/adapter.rs) reset `armed: Vec::new()` on every Elm `Program::update` step, discarding armed timers so subsequent state exits could not emit `TimerIntent::Cancel`, and [`StatechartDefinition`](../../crates/elm-effect-xstate/src/adapter.rs) lacked an `extract_timer` path for routing elapsed timers through `Machine::step_timer`.
   - Host timer bookkeeping lacked a monotonic generation `TimerToken` and clock-deadline remaining-delay restoration (`DEVIATIONS.md` D19), exposing event loops to stale queued wakeup callbacks when re-entering a timed state and restarting full delays after snapshot restore.
   - Transient states entered and immediately exited via `always` in the same macrostep emitted un-netted `[Start, Cancel]` timer intents and `[Spawn, Stop]` actor directives (`DEVIATIONS.md` D17), allowing `ActorSystem` to execute `start_host_actor` and leak `ActorTask`s for transient actors that were already stopped.
3. **Missing XState v6 Primitives, Microstep Observability, and Macro Ergonomics**:
   - `Machine` lacked a zero-mutation `can(&state, &event)` probe, `StateNodeType::Choice` pseudo-states, dynamic transition target functions (`on_fn`) that can decline and bubble to ancestors, event-key labeling (`on_key`), compile-away microstep `Observer` hooks (`step_observed` / `microsteps`), and `to_machine_json()` export for Stately tooling.
   - `ActorSystem` aborted `run()` with `Err(ActorError::Unknown)` when an internal `enq.send_to` or queued inbox envelope targeted an actor that stopped earlier in the step, rather than recording structured `DeadLetter` notices and cleaning up stopped subtrees (`DEVIATIONS.md` D18).
   - Typed machine construction required manual `.with_parent(...)` wiring, repetitive `.label(...)` calls, and redundant enum matching across `.on(...)`, `.guard(...)`, and `.action(...)`.

How should `elm-effect-xstate` adopt `plusthree`'s strongest kernel, timer, observability, and DX invariants while preserving 100% backward compatibility with ADR-0007's `Scope` structured concurrency, `TestClock` determinism, `DurableLog` epoch fencing, `Topology` static checks, and TLA+/TLC formal verification?

## Decision Drivers

* **Preserve Zero Breaking Changes**: All existing `Machine`, `DynamicMachine`, `ActorSystem`, `StatechartProgram`, and conformance test suites must continue to compile and pass unchanged.
* **$O(1)$ Subtree Queries & Pre-Order Topology**: Replace repeated runtime tree traversals with build-time pre-order indexing (`StateIdx`, `subtree_end`, precomputed children/ancestry slices).
* **Eliminate Timer & Transient Actor Races**: Fix the `StatechartModel` armed-timer drop, add generation-fenced `HostTimers` / `DrivenMachine`, support remaining-deadline restoration (D19), and net intra-macrostep timer/actor churn (D17).
* **Zero-Cost Observability**: Microstep observation must compile away completely (`const ENABLED: bool = false`) for `()`, while providing full microstep traces (`MicrostepRecord`) when enabled.
* **Resilient Actor Messaging**: Internal sends to stopped/missing actors become inspectable `DeadLetter` values instead of crashing `ActorSystem::run()`.
* **Ergonomic Declarative Authoring**: Provide a zero-dependency declarative `statechart!` macro alongside the builder API, with automatic parent wiring, single-match pattern destructuring, and transition labels.

## Considered Options

* **Option 1: Point Fixes Only in `adapter.rs` and `actor.rs`**. Patch `StatechartModel.armed` and `ActorSystem::stop` without changing `Machine` internals or adding `HostTimers`, `Observer`, `CompiledIndex`, or `statechart!`. Leaves $O(N)$ tree walks, stale timer wakeup hazards, missing XState v6 primitives (`can`, `Choice`, `on_fn`), and builder boilerplate unaddressed.
* **Option 2: Wholesale Replacement of `elm-effect-xstate` with a `plusthree`-Style Untyped Bitset Kernel**. Rewrite `Machine<S, E, C, CMD>` around `StateId(u32)` and a `#[derive(Context)]` patch macro, breaking `Machine<S, E, C, CMD>` closures (`&mut C`), `Scope` integration, and existing conformance tests.
* **Option 3: Accretive Architectural Hardening via Laws L7–L11 [Chosen Option]**. Introduce `CompiledIndex<S>`, intra-macrostep netting, `HostTimers` / `DrivenMachine`, `Observer`, `DeadLetter` handling, `Choice` / `can` / `on_fn` / `on_key` / `to_machine_json()`, and `statechart!` inside `crates/elm-effect-xstate` while keeping all existing public types and signatures compatible.

## Decision Outcome

Chosen option: **Option 3 ("Accretive Architectural Hardening via Laws L7–L11")**.

We extend [ADR-0007](0007-port-xstate-v6-into-rust-as-elm-effect-xstate.md)'s Laws L1–L6 with five binding invariants (**Laws L7–L11**) in [`crates/elm-effect-xstate`](../../crates/elm-effect-xstate):

### Laws L7–L11

1. **L7: Pre-Order Compiled Topology & $O(1)$ Subtree Invariance (`compiled.rs`)**:
   At `MachineBuilder::build()` time, the state hierarchy compiles into an immutable pre-order `CompiledIndex<S>` assigning every declared node a pre-order `StateIdx(u32)` and exclusive `subtree_end: StateIdx(u32)`, plus precomputed `children`, `ancestry`, `path_from_root`, and `depth`. Checking `is_strict_descendant(child, ancestor)` is $O(1)$ (`ancestor.idx < child.idx && child.idx < ancestor.subtree_end`), and `children()` / `ancestry()` / `path_from_root()` perform zero linear scans or vector allocations per query.
2. **L8: Intra-Macrostep Netting (`machine.rs` — D17 & Transient Timers)**:
   Within a single macrostep's `StepOutcome`:
   - A `TimerIntent::Start { state, duration }` followed by a `TimerIntent::Cancel { state, duration }` for the same state and duration nets away so the host never starts a dead timer. A `Cancel` preceding a `Start` (e.g., `reenter: true` on an active state) is preserved in `[Cancel, Start]` order.
   - An `ActorDirective::Spawn { id, .. }` followed by `ActorDirective::Stop { id }` in the same macrostep (e.g., an `invoke` on a transient `always` state) nets away (along with intermediate `Send` directives to `id`) before `ActorSystem` spawns or starts host tasks. A `Stop` preceding `Spawn` (e.g., `reenter: true` restarting an invoke) is preserved in `[Stop, Spawn]` order.
3. **L9: Generation-Token Host Timers, Elm Armed Continuity, & Remaining-Deadline Restore (`host.rs`, `adapter.rs` — D19)**:
   - `StatechartModel<S, C>` retains `armed: Vec<ArmedTimer<S>>` across `Program::init` and `Program::update`, and `StatechartDefinition::extract_timer` routes elapsed timers directly to `Machine::step_timer`.
   - `HostTimers<S, H>` and `DrivenMachine` assign a unique monotonic `TimerToken(u64)` to every armed timer so queued stale wakeups from an exited or re-entered timer arm are rejected (`fired` returns `None`).
   - `HostTimers::persist()` and `HostTimers::restore_remaining(persisted, now_ms, ops)` record `scheduled_at_ms` and resume with `(scheduled_at_ms + duration).saturating_sub(now_ms)` rather than restarting the full delay.
4. **L10: Zero-Cost Microstep Observability (`observer.rs`, `machine.rs`)**:
   - `Observer<S, C, CMD>` defines `const ENABLED: bool` (`false` for `()`, `true` for `Vec<MicrostepRecord<S, C, CMD>>` and custom observers).
   - `Machine::step_observed` and `Machine::microsteps` emit `MicrostepRecord { kind, exited, entered, configuration, context, commands, timers }` for each microstep (`Init`, `Event`, `Timer`, `Always`, `Done`, `Raised`) in strict Law L1 order, while `Machine::step_state` delegates with `&mut ()` so disabled observation compiles away.
5. **L11: Dead-Letter Actor Resilience & Declarative Macro DX (`actor.rs`, `macros.rs`, `export.rs`)**:
   - In `ActorSystem`, internal `ActorDirective::Send` to an absent actor or queued `inbox` messages targeting a stopped subtree convert into `DeadLetter { sender, target, event }` entries (`ActorSystem::dead_letters`, `take_dead_letters`) instead of failing `ActorSystem::run()`. `ActorSystem::stop` also purges pending `ActorTask`s and `pending` directives for the stopped subtree.
   - `Machine` adds `can(&state, &event)`, `StateNodeType::Choice`, `StateNode::on_fn` (dynamic target selection that declines and bubbles when returning `None`), `Transition::on_key`, and `Machine::to_machine_json()`.
   - The `statechart!` macro provides nested declarative authoring with automatic parent wiring, automatic transition labels/keys, and single-match event pattern bindings.

### Consequences

#### Positive
* Eliminates `StatechartModel` timer cancellation amnesia and stale timer wakeup hazards.
* Prevents transient `always` invokes from spawning or leaking `ActorTask`s.
* Makes subtree checks $O(1)$ and removes runtime `children()` scans and path sorting allocations.
* Bridges `elm-effect-xstate` to Stately tooling via `to_machine_json()` and zero-cost microstep inspection via `microsteps()`.
* Preserves every existing ADR-0007 public API and invariant (`Scope` LIFO, `TestClock`, `DurableLog`, TLA+ verification, `Topology`).

#### Negative / Tradeoffs
* Adds 4 focused modules (`compiled.rs`, `host.rs`, `observer.rs`, `macros.rs`) inside `crates/elm-effect-xstate`.
* Maintaining both the fluent `Machine::builder` API and `statechart!` macro requires ensuring both compile down to the identical `StateNode` / `Transition` IR.

## Implementation Plan

* **Affected paths**:
  - `crates/elm-effect-xstate/src/compiled.rs` (new): `StateIdx`, `CompiledIndex<S>` pre-order table and $O(1)$ `is_strict_descendant`.
  - `crates/elm-effect-xstate/src/observer.rs` (new): `MicrostepKind`, `MicrostepRecord`, `Observer` trait (`ENABLED: bool`).
  - `crates/elm-effect-xstate/src/host.rs` (new): `TimerToken`, `TimerOp`, `PersistedTimer`, `HostTimers`, `HostScheduler`, `DrivenMachine`.
  - `crates/elm-effect-xstate/src/macros.rs` (new): declarative `statechart!` macro.
  - `crates/elm-effect-xstate/src/types.rs`: add `StateNodeType::Choice`.
  - `crates/elm-effect-xstate/src/transition.rs`: add `event_key`, `dynamic_targets`, `Transition::on_key`, `Transition::dynamic`, `Transition::dynamic_targets`.
  - `crates/elm-effect-xstate/src/state_node.rs`: add `StateNode::choice`, `StateNode::on_fn`.
  - `crates/elm-effect-xstate/src/machine.rs`: embed `CompiledIndex`, add `MachineBuilder::choice`, `Machine::can`, `Machine::step_observed`, `Machine::microsteps`, dynamic target resolution in `select()`, and intra-macrostep netting (`net_macrostep_timers`, `net_macrostep_directives`) in `outcome()`.
  - `crates/elm-effect-xstate/src/dynamic.rs`: expose `DynamicMachine::can`.
  - `crates/elm-effect-xstate/src/adapter.rs`: preserve `armed` in `StatechartModel`, add `StatechartDefinition::extract_timer`, route timers in `StatechartProgram::update`.
  - `crates/elm-effect-xstate/src/actor.rs`: add `DeadLetter`, record dead letters for missing/stopped internal sends, purge `tasks`/`pending`/`inbox` for stopped subtrees in `ActorSystem::stop`.
  - `crates/elm-effect-xstate/src/topology.rs`: handle `StateNodeType::Choice`.
  - `crates/elm-effect-xstate/src/export.rs`: handle `StateNodeType::Choice`, add `Machine::to_machine_json` and `DynamicMachine::to_machine_json`.
  - `crates/elm-effect-xstate/src/lib.rs`: export new modules and symbols at root and `prelude`.
  - `crates/elm-effect-xstate/tests/adr_0008_hardening.rs` (new): comprehensive regression & proof suite for Laws L7–L11 and deviations D4, D5, D17, D18, D19.
  - `docs/guides/18-xstate.mdx`: document new primitives, `HostTimers` / `DrivenMachine`, `microsteps`, `DeadLetter`, `to_machine_json`, and `statechart!`.
* **Patterns to follow**: Pure sans-I/O `Machine`, `Scope`-bound `ActorSystem`, `TestClock` virtual time, `#[must_use]` on query methods, concise docstrings on all public items.
* **Patterns to avoid**: Breaking existing `MachineState`, `TimerIntent`, or `TimerFired` signatures/struct literals; adding `use elm_effect...;` imports in `docs/guides/18-xstate.mdx` that are not in `crates/elm-effect-elm/tests/guide_imports.rs`'s `PUBLISHED` set.

### Verification

- [x] `CompiledIndex` passes $O(1)$ descendant and pre-order hierarchy tests across compound and parallel states.
- [x] `Machine::can`, `StateNode::on_fn` (dynamic target + decline bubbling), and `StateNodeType::Choice` pass unit proofs.
- [x] Intra-macrostep netting drops transient `[Start, Cancel]` timers and transient `[Spawn, Stop]` invoke directives while keeping `[Cancel, Start]` / `[Stop, Spawn]` on re-entry.
- [x] History re-entry on parent (`p -> p.h`) consistently restores a valid configuration (D4).
- [x] `StatechartModel` retains `armed` across updates and routes `extract_timer` through `step_timer`.
- [x] `HostTimers` / `DrivenMachine` reject stale `TimerToken` callbacks and compute remaining deadlines on restore (D19).
- [x] `ActorSystem` records `DeadLetter`s for stopped/missing internal targets and purges pending `ActorTask`s on subtree stop (D18).
- [x] `step_observed` / `microsteps` record every microstep in Law L1 order.
- [x] `to_machine_json` exports valid XState v6 JSON and `statechart!` compiles nested labeled charts with payload bindings.
- [x] `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo fmt --check` pass with 0 warnings/failures.

## More Information

- [ADR-0007: Port XState v6 into Rust as elm-effect-xstate](0007-port-xstate-v6-into-rust-as-elm-effect-xstate.md)
- [`rockorager/plusthree`](https://tangled.org/rockorager.dev/plusthree) design & deviations study (2026-11-19)
