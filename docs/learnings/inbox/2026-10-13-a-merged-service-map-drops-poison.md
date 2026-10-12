---
tier: perishable
tags: [elm-effect, services, merge, conflict, ambient]
bead: elm-effect-rs-effect-core-apis-missing-x3dj
created: 2026-10-13
verify: cargo test -p elm-effect --test l5_memo_refs
verify_result: ok (15 passed; 0 failed)
---
# A merged service map silently drops a poisoned tag

## What happened

`Effect::with_fiber` needs the running fiber at interpretation time, so the
spawn seams now publish a `FiberTag` entry in the ambient services map
(`Ctx::spawn_effect`, `Runtime::spawn_effect`). That made the ambient map
non-empty for every effect run through those seams, where before it was empty
and `merged_services` took its fast path (`outer` empty -> `Arc::clone(own)`).

Two existing tests went red:
`ambiguous_tag_is_a_defect` (l5_memo_refs.rs) and the doubled-tag leg of
`memo_map_sequential_vs_stacked`. Both provide the same tag twice with
distinct values and assert the run Dies. With a non-empty `outer`, the merge
path ran and the poison vanished, so the run succeeded.

## The bug

`ServiceMaps::tag_list()` replayed `(id, value)` pairs and dropped the
`conflict` flag. `merged_services` rebuilt the map by inserting those pairs
into a clone of `outer`; `insert_tag` can only spot a conflict BETWEEN the two
sides. A conflict that lived inside `own` (one key, two distinct provisions)
was replayed as a single clean provision and disappeared. `Layer::provide`
did the same to `BuiltServices`. The doc law says the opposite: "any run of
the poisoned build Dies (never last-writer-wins)".

Nothing failed before the ambient existed because the merge path was only
reached when both sides were non-empty, and the tests that exercise poison ran
with an empty `outer`.

## The fix

`tag_list()` now carries the verdict as a third element, and both replay sites
poison the merged entry when it is set. `ServiceMaps::poison(id)` marks an
existing entry without touching its value. The regression witnesses are the
existing tests: they were red under the new ambient, green after the fix.

## Lesson

A replay that carries values but not verdicts loses exactly the property the
verdict encodes. When a map is copied through a projection, every flag the
consumer checks has to survive the projection, or the projector is a
correctness hole that waits for someone to start using the path.
