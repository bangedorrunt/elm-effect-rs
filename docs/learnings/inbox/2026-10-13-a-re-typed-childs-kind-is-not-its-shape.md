---
tier: perishable
tags: [schema, port, repr, kinds, nested-constructors, rust]
bead: elm-effect-rs-port-schema-a2c
created: 2026-10-13
verify: sh -c "! grep -q 'Box::new(inner\\.kind' crates/elm-effect-schema/src/lib.rs && cargo test -p elm-effect-schema --test schema_members -q 2>&1 | grep -q 'test result: ok'"
verify_result: ok (exit 0; the defective read is gone and the eight member witnesses pass)
---

# A re-typed child's `kind` is a marker, not its shape

## The failure

The first run of the `mapMembers` tests was red with
`["Schema.Array(Schema.Unknown)"]` where the pin says
`Schema.Array(Schema.String)`, and `["Schema.Object"]` where it says a whole
`Schema.Struct({ ... })`. The derivation was right; the *rendering* was wrong.

`Schema<T>` carries a shape in two places. `kind` is the shape for a schema
built directly. `Schema::as_unknown` (how every struct field, tuple element and
union member is spelled, since they must share one `Unknown`) stores the real
shape in `wrapped` and sets its own `kind` to `SchemaKind::Opaque("unknown")`.
So a constructor that derives a child kind with `inner.kind.clone()` records the
marker. `Schema::array` did exactly that, which is why an `Array` of anything
re-typed rendered as `Schema.Array(Schema.Unknown)`.

`repr_kind()` is the accessor that resolves this: a wrapped schema answers the
kind it wraps, anything else answers its own. `Schema::record` already used it,
with the comment "a value that arrived here through a re-typing has an opaque
kind of its own". `array`, `as_unknown` (its `Wrapped` snapshot, both its code
and type), `convert`, and the two formatter fallbacks did not.

## Why the tests were the thing that found it

No existing witness compared a *nested* code string built through a re-typing:
`array(inner)` was exercised for decode and encode, where the marker is
harmless, and every struct field code was compared one level down
(`field.inner().to_code()`), which never asks the parent to render its child.
Porting `mapMembers` did, in the same assertion that checks the member list, so
one red test named both the missing derivation and the bad render.

## The recipe

When a port constructor takes a child schema and keeps a shape derived from it,
read the child's `repr_kind()`, never its `kind`. The same goes for a snapshot
that stores code or type: `to_code()`/`to_type()` already resolve a wrapper,
while `code_of(&schema.kind, ...)` records the marker.

## Residual

`format_root`'s `Struct` arm still reads `schema.kind` for its children
(`schema.kind.clone()` where a nested field's shape is wanted). It is reachable
only when a struct carries no formatter of its own, which no constructor in the
port produces today, so no witness covers it. It is the same defect one call
deeper and wants the same one-word change when a row reaches it.

## Principles

- **fix-root-causes** — the failing assertion looked like a wrong derivation and
  was a wrong render. Three constructors shared one cause, so the fix went to
  the accessor's readers rather than to the test.
- **prove-it-works** — the defect was invisible to every decode/encode witness.
  It took an assertion on the derived repr to make the marker visible.
