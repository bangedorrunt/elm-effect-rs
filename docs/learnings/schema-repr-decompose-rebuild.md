# The schema repr is a decompose/rebuild pair, not a printer

Date: 2026-10-13
Bead: `elm-effect-rs-port-fromjsonschemadocument-ad2`

## What we built

`crates/elm-effect-schema/src/lib.rs` carries
`packages/effect/test/schema/representation/fromJsonSchemaDocument.test.ts`
against the Effect v4.0.0 pin. The port's job is to import a JSON Schema
document into a `Schema` and then emit the pin's Source-style `code` and
TypeScript-style `type`, which is what `SchemaRepresentation.toCodeDocument`
produces in the pin.

## The shape that made it work

`to_code` and `to_type` alone were not enough. Every remaining family turned out
to need the schema taken apart and put back together, so the port gained a pair:

- `decompose(kind) -> (base_kind, Vec<Check>, Vec<(String, String)>)` lowers a
  schema into its base shape, the checks layered on it, and the annotations
  riding the base.
- `rebuild(kind, checks, annotation) -> Schema` raises them back.

Everything else is a list operation over that triple: `allOf` intersection,
literal refinement filtering, a document's own `description` landing on the base,
and `makeFilterGroup`.

## The four things worth remembering

**`as_unknown` hides its child, and every constructor that copies a child's kind
must see through it.** `as_unknown` keeps its own kind opaque so the JSON Schema
document does not change, which means `record`, `checked`, `struct_of`,
`tuple_with_rest`, `union_with_mode`, `annotated_with` and `new_kinded` all read
`repr_kind()` rather than `kind`. Getting this wrong prints `Schema.Unknown` for a
perfectly visible child, and it was the same defect three separate times.

**An untyped base is not a base.** `{ allOf: [{ type: "string", ... }] }` has an
empty sibling document once `allOf` is removed, so it imports to the open JSON
value. The intersection defers to whichever side names a type. Decide this on the
DECOMPOSED kind: the open value arrives wrapped in `Annotated`, and a raw-kind
check misses it and then refuses every typed member as a different base.

**`maxLength` is a code-point length and `minLength` is not.** The pin's import
renders `maxLength` as `Schema.isMaxCodePoints` with "a string with at most N
code points" while `minLength` stays `isMinLength` with "a value with a length of
at least N". That asymmetry is the pin's, not a mistake, and both messages name
their unit.

**A key is measured in code points, not characters.** A `minLength` beside
`propertyNames` is a code-point length. Rewrite the document before importing
rather than patching the check afterwards, so the check's own message names the
right unit.

## What is still open, and why

The remaining 136 rows of this file need a `$ref`/`$defs` subsystem (a Reference
and definition model, `$id` scoping, recursion), real union distribution
(`Schema::union` still records `Opaque("union")`, so a member's constituent bases
cannot be read out of it), `patternProperties` lowering over a finite object
domain, and a JS-exact `toFixed` decimal-places model. The last one matters
because JS reports zero decimal places for a number it prints in exponent
notation, which is what the pin's three extreme `multipleOf` rows turn on.

## How to add a row

Write the Rust test against the pin's OWN expected `code` string, copied from the
`it(...)` block, not against a shape you expect. Two rows were wrong the first
time this way and only the pin's literal text caught them: `maxLength`'s check
name, and the fact that `allOf` was being pre-empted by an early `enum` return
so the intersection was silently dropped.
