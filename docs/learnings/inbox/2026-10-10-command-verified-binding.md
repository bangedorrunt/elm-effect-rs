---
tier: perishable
tags: [br, close-ceremony, gates]
bead: elm-effect-rs-cache-late-completion-cma
created: 2026-10-10
verify: flywheel ledger check elm-effect-rs-cache-late-completion-cma --dir .
verify_result: ok (LEGAL on sha 329a6b8)
---
# A loop-runnable fence closes on `command-verified`, and its note must lead with sha=

## Solution
Three rules decide a close, and only the first is documented well:

1. A fence the loop can run (`cargo test …`, `timeout N …`, `sh -c` with a
   timeout) is closed by `command-verified`. `unit-test-verified` is for the
   non-runnable band; on a `feature` bead it is refused with "not legal for
   closing this bead (priority P2); legal: command-verified".
2. The `command-verified` note must LEAD with `sha=<40-hex>` (a
   `receipt=<path>` beside it is what the guard expects). Without the token
   the row reads UNBOUND, `flywheel ledger check` reports "no legal verdict
   bound to sha", and the close is refused.
3. You cannot repair the row after the close: `br gate report` refuses a gate
   write on a closed bead. Re-report BEFORE closing.

`br close` is what closes. `br gate report --to closed` prints
`[in_progress -> closed]` but does not persist the status; a bead can sit
`in_progress` with a pass gate row until the explicit close runs.

## Principles
sequence-verifiable-units — a gate row is evidence only when it names the sha it judges.
prove-it-works — the receipt and the ledger check both point at one sha.
