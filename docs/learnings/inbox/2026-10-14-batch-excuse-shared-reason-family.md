---
tier: perishable
expires: 2026-11-14
provenance: reinforced 2026-10-14
tags: [conformance, ledger, excuses, rows, bd-iqte, wave-1]
bead: elm-effect-rs-sgze
created: 2026-10-14
verify: sh -c "cd /Users/bangedorrunt/workspace/elm-effect-rs && git show HEAD -- conformance/behavior-tests.tsv | grep '^+not-applicable' | awk -F'\t' '{print $NF}' | sort -u | wc -l"
verify_result: ok (exit 0; 36 excused Command rows collapse to exactly 3 distinct reason strings)
---

# A batch excuse is one claim wearing N rows, so expiry fires at family granularity

## The case

`elm-effect-rs-sgze` (HEAD, `9dc8dd4`) excused 36 Command rows and marked the
module matched in one commit. The 36 rows carry exactly **3 distinct reason
strings** (13 advanced-flag-precedence, 9 parent-config propagation, 14
help-doc metadata). The four sibling commits do the same: Request 28 rows on
2 reasons, PubSub 14 on 2, String 13 on 3, HttpClientRequest 9 on 1. Five
commits, 100 rows, 11 claims.

## Why it matters beyond the expiry learning

The 2026-10-13 expiry learning treats each `not-applicable` row as a claim
that can go stale. Batch excuses tighten that: the claim is written once and
pasted N times, so when the capability lands, **N rows flip together and the
audit that missed the one reason missed all N**. The shared string is both the
efficiency (review one reason, not 36 rows) and the blast radius.

## The rule for later wave-1 crews

When you land a capability, grep the ledger for its excuse's reason substring
and re-check every row wearing it, not just the row in front of you. When you
write a batch excuse, pick the reason string as if it will be grepped later,
because it will be: one stable phrase per capability gap, no per-row
paraphrase that fragments the family.
