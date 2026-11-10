# XState differential check

Runs the crate's conformance fixtures through upstream XState v6 and compares
the resulting machine state. This is an independent oracle for the pure
machine core; it is not a vendored copy of the ~400-case upstream corpus.

## Run

```bash
npm ci
npm run check
```

`check` compares final active leaves, `done`, and final context for 37 cases.
It deliberately does not compare action traces. The npm
`xstate@6.0.0-beta.1` build evaluates inline transition functions during
selection and queues `enq` effects after entry, while this crate's L1 contract
is exit, transition actions, entry. `npm run check:traces` prints those
differences for inspection; trace ordering remains covered by
`cargo test -p elm-effect-xstate`.

Excluded:

- `after.json`: clock-driven; the Rust suite proves it through `TestClock`.
- `actors.json`: uses the actor runner schema, not the machine fixture schema.
- `emitted_payloads_reach_inspection`: emitted events are not exposed on the
  snapshot the harness reads.

The six upstream-derived vectors cite
`xstate@6.0.0-beta.1 commit d56483f` in their `source` field. The npm version
and the source commit share a version string but are built separately, which
is why trace ordering is not asserted here.

The workflow in `.github/workflows/xstate-differential.yml` runs this check
only when the fixtures or harness change. It is separate from the bare-cargo
`ci` workflow.
