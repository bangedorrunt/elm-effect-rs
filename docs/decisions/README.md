# Architecture decision records

`accepted` is the operator sign-off. `proposed` is not a stop when the record has an agent-authority section. ADR-0001 gives the rule.

Numeric prefixes are stable. A superseded record keeps its number. The new record links back.

| Record | Status | Decision |
| --- | --- | --- |
| [0001](0001-adopt-architecture-decision-records.md) | proposed | Adopt architecture decision records in this directory |
| [0002](0002-port-effect-4-into-rust-with-an-elm-loop.md) | proposed | Port Effect 4.0.0 into a Rust carrier with an Elm loop and a durable-stream log. Agent procedure: `skills/elm-effect` |
| [0003](0003-make-elm-effect-a-first-class-rust-framework.md) | proposed | Make elm-effect a first-class Rust framework, with effect-native extension |
| [0004](0004-reconcile-and-file-backed-log.md) | proposed | Reconcile uncertain outcomes with the journal planner, and back the loop log with a file |
| [0005](0005-durable-engine-port-and-our-engine.md) | proposed | Durable engine port (scheduling verbs) plus our own single-host engine; it replaces Resonate, with no vendor adapters |
| [0006](0006-integrate-harel-statecharts-into-the-elm-loop.md) | proposed | Integrate Harel statecharts and actor lifecycles into the Elm loop, binding state lifetimes to structured scopes without a second scheduler |
