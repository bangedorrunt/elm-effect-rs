# Scheduling effect steps in the schema parser

Three traps that cost real time landing the bounded-concurrency rows
(`SchemaParser.test.ts:84` through `:247`).

## A parked step is woken by whoever polled it

`FanOut` polls every child with the parent's context, so a step that parks its
thread and later wakes registers the *outer* waker (the `block_on` `Unpark`).
That is what makes a gate opened from the test thread resume the fiber. It also
means a child that returns `Pending` without registering a waker is lost for
good; every `Wait`-style future here must push the waker every poll it fails.

## The encode axis needs its own gate

The row loops `for (decode, encode)`. A fixture whose *decode* step waits on a
gate but whose *encode* step answers immediately proves nothing about the encode
axis: `resolve` runs the encode step, it never touches a gate, and the test
times out waiting for a child to start. The step factory has to gate both
directions (`gated_step` builds one closure used twice).

## Completion order decides a record key collision

Two wire keys that decode onto one name (`a_b` and `aB` through a camelizing
key schema) are assembled by insertion as each child completes, so the last to
finish keeps the name. Do not sort before inserting: that would make the row's
answer the declaration order the pin explicitly rejects.

## Worth knowing

- `Schema::as_unknown` copies `decode_step`/`encode_step` and every structural
  slot, so a re-typed effect codec still reports `has_effect_steps() == true`
  and its step still runs. A wrapper that dropped either would make a step
  invisible to the walk.
- Which gate name can authorize a close is per bead, and the tool tells you:
  this P2 bead with a runnable VERIFY fence accepted `command-verified` and
  warned that `unit-test-verified` "will not authorize the close". Report the
  gate anyway, read the warning, and report the legal name. The verdict note
  must carry `sha=<full sha>` of the commit that cites the bead, so gate after
  the commit, never before.
- `cargo fmt --check` fails on ~80 files in this tree already; formatting is not
  a gate here, and reformatting a touched file wholesale buries the diff.
