---
tier: perishable
tags: [elm-effect-rs, schema, wire-text, fromJsonString, unknown]
bead: elm-effect-rs-cchs
created: 2026-10-19
verify: sh -c "cargo test -p elm-effect-schema --test schema_wire && cargo test -p elm-effect --features schema --test schema_duration"
verify_result: unit-test-verified (sha=3796dee; fence green)
---

# Wire-text schemas: the number spelling, and why a decoding transform cannot nest in a Struct

`Schema.fromJsonString`, `UnknownFromJsonString`, `Trim`, `DurationFromString` and
`DurationFromMillis` landed under `elm-effect-rs-cchs` at `3796dee`. Two mechanisms
cost real time and will cost the next porter the same.

**A JSON number prints the JavaScript way, and only the printing side knows it.**
The row wants `JSON.stringify({ a: 1 })` to be `{"a":1}`, while `Unknown::to_json`
maps a number through `serde_json::Number::from_f64`, which spells it `1.0`. So the
text-printing path converts through `web_json` (integral and inside the safe range
becomes an integer, non-finite becomes `null`) instead of `Unknown::to_json`. The
trap: the replacer hook also re-serializes the value it was handed, so it needs the
same conversion — leaving `to_json` there re-floats everything the hook touches, and
the failure shows up in the *space/replacer* row, not the plain one.

**`as_unknown` is the wire form, not a type cast.** A struct field can only be
spelled `Schema<Unknown>`, so re-typing a schema re-encodes through it: the
wrapper's decode is the inner ENCODE (`schema-repr-decompose-rebuild.md` says the
same from the repr side). A decoding transform therefore comes back as its own text
instead of the parsed value, which is why `Schema.test.ts:6071`
(`Struct({ a: fromJsonString(Struct) })`) stays unported: the row wants
`a: { b: 2 }` and the port can only give `a: "{\"b\":2}"`.
`as_unknown(NumberFromString)` behaves the same way, so this is the port's model,
not one schema's bug.

**Ceiling, stated rather than worked around.** `Schema.test.ts:5711`
(`DurationFromNanos`) is excused: its wire type is a bigint and `Unknown` has no
arbitrary-precision integer variant — the same gap that excuses
`Formatter.test.ts:66` (`crates/elm-effect/src/json.rs:19`). If such a variant ever
lands, the constructor is one `convert` call plus the encoding-fails-on-infinity
half, which needs a fallible-encode constructor the port does not have yet.
