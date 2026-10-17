# The Bend carrier (elm-effect-bend)

Repo root: `/Users/bangedorrunt/workspace/elm-effect-bend`. The consumer it was
shaped for is `~/workspace/bendx` (bird, the X CLI).

Bend has no async runtime, no exceptions, no fibers and no scopes. So the Rust
carrier's shape does not transfer: what carries the laws here is a `Data` tree
plus a pure scheduler and one interpreter. Read this file, then the repo's
`docs/guides/00-overview.mdx` and `docs/guides/07-production-readiness.mdx`.

## The mapping

| Rust carrier | Bend carrier | File |
| --- | --- | --- |
| `Effect<A, E, R>` | `E.Eff<a, M>` (atoms + `link`, `retry`, `fallback`, `bracket`, `keyed`, `deadline`, `durable`) | `src/eff.bend` |
| typed `E` | `O.Outcome`: ok, fail(code), die, cancel, park | `src/outcome.bend` |
| service traits / `R` | the app's own `Model`/`Msg`, defs handed in as erased (`~`) parameters, and `T.TPort` for out-of-process modules | `src/program.bend`, `src/cli.bend` |
| the executor | `Pl.Plan.next` / `Pl.Plan.settle` (pure) + `Loop.run` (one effect per turn) | `src/plan.bend`, `src/loop.bend` |
| the platform layer | `Run.perform`, `Run.timing`, the journal file. The only impure module | `src/run.bend` |
| `DurableStream` | `L.Log`: offsets, `(producer, epoch, seq)`, `Row`, key index, replay | `src/log.bend` |
| `Program::run` and exit status | `Cli.boot`, `Outcome.exit_code`, `IO.die` in `src/cli.bend` only | `src/cli.bend` |

## Gates

- `bend PROOF.bend` must print `ALL PROOFS CHECK`; `bend PROOF.bend --verdict`
  re-checks through the kernel. Every `law` needs `def Laws.<name>`.
- `bash tests/e2e.sh` (real command line, transcript committed),
  `bash tests/http_e2e.sh` (real socket), and the in-process files:
  `tests/world_seam.bend`, `tests/seam_pairs.bend`, `tests/log_index.bend`,
  `tests/log_codec.bend`, `tests/clock.bend`.
- `BEND=/path/to/bend bash ...` when `bend` is not on `PATH`; the suites default
  to `bend`.

## Port order for bird

1. `Msg` and `Model` as they are; the fold's return becomes `G.Step<Model, Msg>`.
2. `Loop.run` with bird's defs, keeping bird's `src/effects.bend` interpreter
   until the run is green.
3. `Run.perform` task by task, diffing `Run.http`'s argv against bird's
   `Http.assemble`. Two argv builders is the drift to avoid.
4. Durable keys last: they buy replay and they commit the app to idempotency
   keys (a settled failure stays settled).

## What the Bend carrier does not give you

- **No scope-based cleanup and no cancellation of children.** `bracket` and the
  `Release` atom cover acquire/release; a deadline yields `OutCancel`, not an
  interrupt delivered into the middle of a step. There are no child tasks.
- **No parallelism.** One effect per turn. Fan-out is a design change.
- **No power-loss durability.** `File.open` has no `fsync`, no exclusive create
  and no lock. Recovery is honest for a process crash (intent before effect);
  the tail of a file can still be lost.
- **No version to pin.** The repo has no tag; cite commits and `CHANGELOG.md`.

## The Bend-specific traps, each measured here

- **Callee before caller.** Forward references fail as "a filled definition".
- **No mutual recursion.** The scheduler is two independent recursions.
- **`Bool.pick` is eager.** Every branch is built; guard a parse behind a
  `match` on a `Bool` parameter or an unrelated branch runs it.
- **A `+`-marked def is a different type** from the plain arrow a seam expects,
  and a continuation's last parameter may not be marked `+`.
- **An erased slot takes comptime values only**, so seams come in pairs and the
  twin must delegate (see `Seam pairs` in `docs/claims.mdx`).
- **`IO.now()` is milliseconds.** Dividing it as nanoseconds made every elapsed
  time zero, so no deadline could fire. `tests/clock.bend` and
  `tests/deadline_app.bend` pin it.
- **A row codec that refuses a field repeats the effect.** Escape fields; keep a
  payload's kind in its own column; a nine-field row from before the kind column
  still reads as text.
- **`IO.die` skips cleanup.** Return a run value and let `src/cli.bend` exit.
