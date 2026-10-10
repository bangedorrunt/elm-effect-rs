---
tier: perishable
tags: [conformance, ledger, beads, cargo, wrapper, flake]
created: 2026-10-11
verify: /Users/bangedorrunt/.cargo/bin/cargo test -p elm-effect --test race
verify_result: flaky by design (see §3)
---

# Three traps that will poison every remaining wave-1 bead's fence

## 1. `cargo` in this shell is a mise wrapper that exits 101 on green tests

`which cargo` resolves to `~/.local/share/mise/command-wrappers/bin/cargo`, a
symlink into `mise`. On a fully green run it prints every `test result: ok`
line and still exits **101**, because the mbx build cache cannot map the
temp target dirs its own scratch tests create
(`result was not stored: absolute path has no stable cache mapping`).

So `cargo test -p elm-effect && python3 conformance/rows-port-remaining.py`
looks like a fence failure when nothing failed. The first half exits 101, the
`&&` short-circuits, and the second half never runs.

**The real binary is `~/.cargo/bin/cargo`.** Use it for any exit-code claim.
This cost a misdiagnosis here: I read the wrapper's exit 101 as a real failure
and spent a run chasing it.

## 2. A bead was closed while its own fence still failed

`elm-effect-rs-port-metric-525` is now `closed`, `close_verdict:
unit-test-verified`, `commit_sha 540eaee`, `close_reason: "done"`. That sha is
my *first* commit only, which ported **14** rows. Ledger now reads **41 of 67
rows still unported**, and the bead's VERIFY
(`cargo test -p elm-effect && python3 rows-port-remaining.py
packages/effect/test/Metric.test.ts`) exits **1**, citing 41 open rows.

The close is not defensible. Note that `br doctor` already reports
`gate.verdict_orphans`: **128 closed issues carry a close_verdict with no
gates.jsonl PASS row** that proves the close. So this is a pattern, not one
bad close.

The gate row that does exist for this bead (id 6, provider `CoralHarbor`)
names 14 test functions and is true about those 14. It is false about the
other 41. A gate note that is *true about what it cites* can still be false
about what it claims.

**Do not treat a closed bead as verified.** Run its VERIFY verbatim before
believing any close, mine included.

## 3. `race_all_first_completes_on_the_first_completion_even_a_failure` is a
pre-existing flake

`cargo test -p elm-effect` exits **101** roughly one run in five, and the
failure is always `crates/elm-effect/tests/race.rs:104`,
`timed out waiting for: the failing body parked`, inside `wait_until`'s
10-second deadline.

It is **not** caused by the metric change. Proven: an isolated worktree at
`83211e8`, the commit before this branch's first commit, flaked on run 22 of
25. `--no-fail-fast` does not rescue it, because it is a timing sensitivity,
not a cascade.

Before attributing a red `cargo test -p elm-effect` to your change, run it a
few times and check whether the only failure is this test.

## Principles

- **prove-it-works** — "done" is the exit code of the bead's own fence, run
  through the unwrapped cargo, read from the terminal. Not a status field. And
  a single green run is not a green suite when the suite is known-flaky.
- **fix-root-causes** — 101-vs-0 has one root cause (the wrapper), so the
  remedy is "use the real binary", not per-command escaping. The flake has a
  different root cause and needed a different remedy: an isolated worktree at
  the parent commit.
- **sequence-verifiable-units** — a unit that is closed while 41 of its rows
  are still `unported` teaches the ledger to lie; the row count is the only
  authority.
