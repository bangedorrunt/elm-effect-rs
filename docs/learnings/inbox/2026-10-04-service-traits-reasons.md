---
tier: perishable
tags: [bd-iqte, platform, wave-11]
bead: elm-effect-rs-epic-elm-effect-port-0c6.53
wave: 11
created: 2026-10-04
verify: cargo test -p elm-effect --test platform_config_crypto
verify_result: ok (21 passed)
---
# Services keep tests deterministic; reasons do not survive regen

## Solution
Crypto and config behind traits/structs with fakes means the fence
never touches the network, disk, or environment (one from_env test
excepted, with unique var names). `modules.toml` reason lines are
rewritten by inventory regeneration: only status flips survive, so
residual context belongs in the close report and thread, never in the
reason field.

## Principles
prove-it-works — every flipped row names a test.
foundational-thinking — service trait, no live handles.
