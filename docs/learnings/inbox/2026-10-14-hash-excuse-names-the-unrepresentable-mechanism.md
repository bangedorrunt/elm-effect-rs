---
tier: perishable
expires: 2026-11-14
provenance: reinforced 2026-10-14
tags: [conformance, ledger, excuses, rows, bd-iqte, wave-1]
bead: elm-effect-rs-port-hash-jwgp
created: 2026-10-14
verify: sh -c "cd /Users/bangedorrunt/workspace/elm-effect-rs && python3 conformance/rows-port-remaining.py packages/effect/test/Hash.test.ts"
verify_result: ok (exit 0; "all 25 rows ported or excused")
---

# A batch excuse passes review only when each reason names the unrepresentable mechanism

## The case

HEAD (`c42378a`) excused 21 Hash rows and marked the module matched in one
commit. The 21 rows collapse to exactly **4 distinct reason strings**
(15 collision-grid, 4 CollisionNode cache/invalidation, 1 throwing-hasher
visited-reset, 1 cyclic-sentinel guard), and `rows-port-remaining.py`
reports zero open for `packages/effect/test/Hash.test.ts`.

## Why this batch survives the gate that Metric/Atom failed

The steer review (W1-F1/W1-F2) refused closes whose `close_reason` claimed
zero open while rows remained, with `do_not_broaden_into: do not mass-excuse`.
The Hash reasons pass because each one states a **substrate non-representability**,
not a test shape: hashing delegates to `std::hash::Hash` (SipHash) so Effect's
Murmur-combine collision grids are implementation artifacts; the `Hash` trait
has no exception channel so throw-propagation/visited-reset cannot be expressed;
the trait is non-recursive so cyclic-sentinel caching is a platform-substrate
concern. A reviewer can falsify each claim by pointing at a representable API;
a generic "surface-missing" bucket cannot be falsified at all.

## The rule for later wave-1 crews

When you write a batch excuse, end every reason string with the mechanism that
makes the behavior unrepresentable in Rust (the delegating trait, the missing
channel, the absent bookkeeping), never with the shape of the test you skipped.
One falsifiable mechanism per family is what lets `matched` stay derived without
becoming parity theater.
