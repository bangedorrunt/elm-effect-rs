---
tier: perishable
tags: [conformance, ledger, inventory, flywheel, beads]
bead: elm-effect-rs-vendor-effect-pin-5qi
created: 2026-10-10
verify: cargo test -p elm-effect-conformance --test ledger
verify_result: ok (2 passed, 1 ignored)
---

# Four traps in the conformance tooling, and one in dispatching against it

## Solution

**1. `inventory.sh` links a module to its test by path, and the paths do not
agree.** It strips `packages/effect/src/` and `packages/effect/test/`
independently, so a module and its test meet only when they share a
subdirectory. `src/Schema.ts` is tested from `test/schema/Schema.test.ts`
(keys `Schema` vs `schema/Schema`); `src/testing/TestClock.ts` from
`test/TestClock.test.ts`; the seven `src/http-api/*` modules have their tests
in `test/httpapi/`. 20 port modules read `tests = []` while their rows sat in
the ledger.

**2. A second filter compounded it.** The link scanner ran `find -name
'*.test.ts'`, but the persistence and MCP-conformance suites name files
`FooTest.ts` and `Foo.test-utils.ts`. The behavior scanner reads every `.ts`,
so those rows were always witnessed — only the link was blind.
`src/process/ChildProcessSpawner.ts` and `src/persistence/RateLimiter.ts` were
invisible for this reason alone.

`port_like_unmatched()` counts a module with no linked rows *forever*, so the
phase-13 gate could not close however many rows were ported: 296 unmatched,
164 of them unmatchable. After both fixes: 294 and 142, `matched` 73 → 75.

**3. The basename fallback must not steal.** `test/ai/Model.test.ts` imports
from `effect/ai`, so it is `src/ai/Model.ts`'s. A fallback keyed on the
basename hands it to `src/schema/Model.ts` too, and then two modules claim one
test. The fix collects every relative-key claim *first* and only falls back to
an unclaimed, unambiguous basename; `Model` stays unlinked rather than
mislinked. Check this deliberately, because a mislink looks exactly like a win.

**4. A VERIFY fence passed through `br`'s TOML and then `sh -c` does not
survive a regex.** A Python one-liner with a character class arrived as a
`SyntaxError: closing parenthesis ']' does not match '('`. `--description-file`
already exists for the same reason. The fix is a committed script the fence
calls by name, which also makes the proof re-runnable by anyone.

## Principles

- **sequence-verifiable-units** — a link the generator cannot see is a module
  that can never claim parity, so the generator must see every test file the
  pin has, under every naming convention the pin uses.
- **prove-it-works** — the linking check is a script run against both the old
  and the new `modules.toml` (red then green), not a claim in a commit message.
- **dont-let-a-fallback-look-like-a-win** — a weaker lookup that quietly claims
  a stronger one's row produces a green ledger and a wrong mapping; assert the
  negative case (`Model` stays unlinked) beside the positives.

## Also worth knowing

**5. `flywheel loop-mode run <project> --work` is a monitor, not a spawner.**
It "feeds queued instructions to idle/finished agents". With no live fleet it
classifies every bead not-ready and tries to mail the operator an alert, which
also failed here because `bangedorrunt` is not a registered recipient in the
project. `--robot-status` showed exactly one agent — the session asking the
question. Getting real dispatch requires spawning a swarm, which is a
spawn-class route and needs the operator's approval of the crew proposal
first. Do not read "no live fleet" as "the loop will create workers".
