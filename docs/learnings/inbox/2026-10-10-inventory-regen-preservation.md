---
tier: perishable
tags: [conformance, ledger, inventory]
bead: elm-effect-rs-tke
created: 2026-10-10
verify: cargo test -p elm-effect-conformance && sh -c 'bash conformance/inventory.sh /Users/bangedorrunt/.cache/effect-4.0.0 >/tmp/inv.log 2>&1 && awk -F"\t" "NR>2" conformance/behavior-tests.tsv | grep -c it.each'
verify_result: ok (ledger 9970 rows, 44 it.each; regen byte-identical twice)
---
# A regen must carry every hand-written column, and chained calls are rows

## Solution
Two invisible losses lived in `conformance/inventory.sh`. The declaration
regex listed bare forms only, so every chained call (`it.effect.each`,
`it.effect.skipIf`, `it.live.prop`) never reached `behavior-tests.tsv`:
49 rows the ledger could not witness. The scanner now matches any
`it.<modifier>` chain, vetoes non-declarations (`it.isFailure(`), and
records the call form as kind (a chained `.each` row is `it.each`).

The same script wiped 13 hand-written `reason` notes in `modules.toml`
("no surface yet", port fences) because only `status` survived a regen.
`reason` is now preserved the same way. Proof for both: regen twice and
diff bytes; diff old and new rows keyed on `(path, line, title)` to show
no status or rust column moved.

A skipIf condition is code (`version !== "2025-11-25"`), so the title is
what follows the argument list, and `take_title` now prefers the quote
character the line opens with, keeping template titles whole.

## Principles
sequence-verifiable-units — the ledger must see every row the pin has, or a port cannot be witnessed.
prove-it-works — regen idempotence is a byte diff, not an inspection.
