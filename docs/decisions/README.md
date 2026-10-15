# Architecture decision records

`accepted` is the operator sign-off. `proposed` is not a stop when the record has an agent-authority section. ADR-0001 gives the rule.

Numeric prefixes are stable. A superseded record keeps its number. The new record links back.

| Record | Status | Decision |
| --- | --- | --- |
| [0001](0001-adopt-architecture-decision-records.md) | proposed | Adopt architecture decision records in this directory |
| [0002](0002-port-effect-4-into-rust-with-an-elm-loop.md) | proposed | Port Effect 4.0.0 into a Rust carrier with an Elm loop and a durable-stream log. Agent procedure: `skills/elm-effect` |
| [0003](0003-make-elm-effect-a-first-class-rust-framework.md) | proposed | Make elm-effect a first-class Rust framework, with effect-native extension |
| [0004](0004-reconcile-and-file-backed-log.md) | proposed | Reconcile uncertain outcomes with the journal planner, and back the loop log with a file |
