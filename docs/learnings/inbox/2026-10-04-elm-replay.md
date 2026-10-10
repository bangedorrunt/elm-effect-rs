---
tier: perishable
tags: [bd-iqte, elm, wave-8]
bead: elm-effect-rs-epic-elm-effect-port-0c6.40
wave: 8
created: 2026-10-04
verify: cargo test -p elm-effect-elm fold_is_deterministic
verify_result: ok (plus 5/5 replay fence)
---
# Replay compares command intentions, never re-runs providers

## Solution
Time travel re-folds logged messages from the latest snapshot and
compares each step's serialized command list with the recorded one;
divergence is a value, not a silent pass. Providers never re-execute
because their results are already messages. Middleware stays
`fn(&Msg) -> Msg` so sagas cannot be expressed by accident.

## Principles
prove-it-works — determinism and divergence both fenced.
foundational-thinking — one log, one fold.
