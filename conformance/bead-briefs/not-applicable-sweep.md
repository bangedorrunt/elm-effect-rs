# Disposition EffectEager and HttpBody rows as not-applicable

Source rows:
- `packages/effect/test/EffectEager.test.ts` (41 rows): `Effect.fnUntracedEager` eager sync-evaluation. Rust `elm-effect` has no eager evaluation variant; effects are values evaluated by the runtime.
- `packages/effect/test/http/HttpBody.test.ts` (5 rows): `file`/`fileFromInfo`/`HttpBodyError`. Rust `HttpBody` deliberately models only Empty/Bytes/Text/Form (File needs a filesystem + range validation).

Both are written per-row reasons via `conformance/flip-row.py --not-applicable`. No Rust code change. No modules.toml change (EffectEager unlinked; HttpBody.ts stays unmatched with 0 ported).
