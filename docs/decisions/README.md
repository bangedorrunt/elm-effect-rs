# Architecture decision records

Records in this directory are binding only when their status is `accepted`. `proposed` means the operator has not signed off. Agents implement `accepted` records and leave `proposed` ones as specifications to review.

Numeric prefixes are stable. A superseded record keeps its number. The new record links back.

| Record | Status | Decision |
| --- | --- | --- |
| [0001](0001-adopt-architecture-decision-records.md) | proposed | Adopt architecture decision records in this directory |
| [0002](0002-port-effect-4-into-rust-with-an-elm-loop.md) | proposed | Port Effect 4.0.0 into a Rust carrier with an Elm loop and a durable-stream log. Agent procedure: `skills/elm-effect` |
