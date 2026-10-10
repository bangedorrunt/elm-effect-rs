---
tier: perishable
tags: [bd-iqte, foundation, numeric, wave-5]
bead: elm-effect-rs-epic-elm-effect-port-0c6.23
wave: 5
created: 2026-10-04
verify: cargo test -p elm-effect --test port_numeric
verify_result: ok (50 passed; re-run exit 0)
---
# Decimals want mantissa/scale, and gates differ by priority

## Solution
BigDecimal as BigInt mantissa plus scale covers normalize, exact-or-100-digit
truncated division with half-up tail rounding, all ten rounding modes, and
plain/scientific rendering. P2 beads close on `command-verified`, not
`unit-test-verified`: the wrong gate records but never authorizes.

## Principles
prove-it-works — every row names a running test, TS divide table verbatim.
sequence-verifiable-units — flips survive inventory regeneration.
