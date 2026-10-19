# A check reads the value its transform produced

Date: 2026-10-19
Bead: `elm-effect-rs-schema-check-view-8bsl`

## The problem

The pin's `check` lands on the target side of a codec, so
`FiniteFromString.check(isInt())` refuses `"1.2"` by reading the number the
transform produced. This port's checks read the `Unknown` they were handed, so
`isInt` saw `"1.2"` as text, could not decide it, and passed it.

## The shape that made it work

`Schema` carries a `check_view: Option<CheckView>` — a pure `&Unknown -> Unknown`
projection from the wire value to the value a check reads. Three rules:

- The typed transforms state it (`finite_from_string`, `number_from_string`),
  because that is the only level where the typed value still exists: the
  `Schema<f64>` decode returns a number, while `as_unknown` normalizes it back
  to the wire text.
- It propagates through anything that rides the same codec: `annotate`,
  `as_unknown`, `checked` (so a second check stacked on the first reads the
  same value) and the filter group.
- Every check site calls `check_value(view, wire)`, which returns the wire value
  when there is no view. One rule, not one per constructor.

A projection never fails: a value the transform cannot read comes back as it
is, and the codec refuses it a moment later with its own message.

## Two traps

**Refusals from a check are a second error variant.** A failed check carries the
check's own `expected` text and the produced value's kind, so `checked` emits
`TypeMismatchDynamic` rather than `TypeMismatch`. Every place that re-roots a
path has to name that variant too: `SchemaError::at` (used by fields, tuple
elements, links and property names) and the two hand-rolled path rewrites in
`field`/`array`. Forgetting one leaves a check inside a struct reporting at `$`
instead of `$.a`, which is exactly what three existing `schema_core` rows
caught.

**`as_unknown` erases the typed value.** Its decode is decode-then-encode, so
`as_unknown(FiniteFromString)` is the identity on the wire. That is why the view
cannot be derived from the wrapper's own decode, and why `:2662` (a check on the
*output* of a flipped transform) stays unported: after the re-typing there is no
text for `isMinLength` to measure. `:2791` also stays unported, for the separate
reason that this port has no `decodeTo` link.

## Verified

`cargo test --workspace --all-targets` green. Five mutations, each killed by its
intended witness and restored byte-identical: drop the `finite_from_string` view
(3 failures), make `check_value` ignore the view (4), drop the box's decode step
(3), drop `at`'s dynamic arm (2), drop `checked`'s propagation (1).
