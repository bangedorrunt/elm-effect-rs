---
title: Effect steps in Schema, and the parser options seam, 2026-10-24
status: verified
---

# Three traps, each found by a test rather than by reading

## A metadata question must not run the thing it asks about

`Schema::has_effect_steps` is a walk over a schema's children. Teaching it to
resolve a `suspend` thunk looks harmless, and it is not: the adapters ask the
question before they parse, so the factory ran at adapter time. Two already
ported laziness rows (`SchemaParser.test.ts:1027`, `:1045`) failed by counting
one extra factory call, and the retry row counted three where it expected two.

Rule: asking whether a schema *carries* a step is inspection. A step under a
`suspend` is found by the parse that resolves the thunk, not by the walk. Same
shape as the older rule that an audit must not compile what it audits.

## A `convert` that keeps the kind but not the slots is an empty object

`Schema::convert` copied the inner schema's `SchemaKind` and nothing else. So
`convert(struct_shape(...), ...)` reported kind `Object` with no field list, and
every structural reader (the `Config.schema` bridge, a JSON Schema document)
saw an object with no properties. The `Config.schema` nested-URL row failed with
`Missing { path: [] }`, which names the symptom, not the cause.

The structural slots are the shape; the kind is only a label. Anything that
re-types a schema has to copy `struct_fields`, `tuple_elements`,
`union_members`, `array_element`, `record_key`, `record_value` and `children`.

## A close gate's legal name depends on the bead, and its note needs the sha

`br gate report --gate command-verified` is recorded but explicitly refused for
a P2 bead; the legal set there was `unit-test-verified`, `live-verified`,
`worker-receipt`, `triage-verified`. A close verdict also reads its note for a
`sha=<full sha>` token, so the order is commit → gate with the sha → close. A
gate reported before the commit exists is unbound and unrepairable after the
close.

# What shipped

- `ParseOptions { disable_checks, report_input }`, honored on both axes, and a
  `_with` twin of every parser adapter. `reportInput` rides
  `SchemaError::WithInput` at the occurrence the entry point named; a nested
  leaf cannot carry its own input, because the port's issue is a typed enum.
- `Schema::effect_codec` with `StepFuture`/`EffectStep`, resolved sequentially by
  the effect adapter over struct, tuple, array and record. Concurrency
  (`ParseOptions.concurrency`, interruption, completion order) is the remaining
  gap and the ledger says so on all eight concurrency rows.
- `Config.schema` completed: `IntoConfigValue for Url` (in `elm-effect-unstable`,
  behind its new `schema` feature) and a field-wise `BTreeMap` impl. Both config
  rows flipped.
