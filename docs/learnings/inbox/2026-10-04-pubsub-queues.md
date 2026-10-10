---
tier: perishable
tags: [bd-iqte, concurrency, wave-7]
bead: elm-effect-rs-epic-elm-effect-port-0c6.38
wave: 7
created: 2026-10-04
verify: cargo test -p elm-effect --test pubsub_subscription
verify_result: ok (34 passed)
---
# Per-subscriber queues beat STM clones; atomicity needs the lock held

## Solution
PubSub as hub plus per-subscriber queues with strategy on publish and
replay windows covers 36 rows with threads standing in for fibers.
SubscriptionRef read-modify-write must hold the cell lock across `f`
(non-reentrant): get-then-set loses updates under contention and the
suite catches it only under parallel load.

## Principles
prove-it-works — every row names a running test, replay/append verbatim.
sequence-verifiable-units — flips survive inventory regeneration.
