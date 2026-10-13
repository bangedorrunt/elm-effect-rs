# Migration skew between the embedded toron and the store it reads

Date: 2026-10-13
Bead: `elm-effect-rs-supervise-preflight-blocked-0e9d`

## Symptom

`flywheel loop-mode supervise <p> --work` never starts. Preflight fails one
check and stops after three consecutive failed passes, so there is no durable
supervisor and the fleet has to be hand-fed:

    FAIL toron-mail — ... migration 18 is applied in
    ~/.local/share/toron/index.sqlite but this toron build resolves at most 17
    ... (toron-error=store_skew)

`flywheel send --wait` keeps working the whole time, which makes it look like
mail is fine. It is not: `send` reaches the standalone toron, while flywheel's
IN-PROCESS mailbox is what the preflight and `loop-mode` read, and that is what
cannot open the store.

## Cause

Three repos share one store, and only one of them had moved:

- `~/.cargo/bin/flywheel` embeds `toron-core` from `~/workspace/toron` by path.
- Local `~/workspace/toron` was 9 commits behind `origin/main`, at a HEAD whose
  `crates/toron-core/migrations` stopped at `0017`.
- The store had already been migrated to `0021` by a toron built from
  `origin/main`.

So the store was 4 migrations ahead of the library flywheel links against. The
error text says "migration 18" because it reports the FIRST one it cannot
resolve, not the store's actual head.

## Fix

    cd ~/workspace/toron
    git merge --ff-only origin/main          # 9 commits; adds exactly 0018-0021
    cd ~/workspace/flywheel
    cargo install --path . --force           # relink against the new toron-core

The fast-forward is safe to recognise by what it adds: the four new migration
files are exactly 0018-0021, matching what the store already carries. If the ff
had brought different migrations, it would have been the wrong branch.

`cargo install` matters over `cargo build`: the installed binary is what every
crew invokes, and it must be a release build. Verify with `flywheel toron status`
before dispatching anything, which reports whether the archive and bus both
open.

## What this is NOT

`--skip-preflight` is operator-only. A build gate that refuses on a genuinely
unreadable store is doing its job; the fix is to make the store readable, not to
make the check quieter.

## Tell

`toron` and `flywheel` and `elm-effect-rs` are three repos on one SQLite store.
Any time one is rebuilt or rebased, check the other two's migration floor before
assuming a mail failure is a code bug. Read the floor from the store itself, not
from the error message:

    python3 -c "import sqlite3; c=sqlite3.connect(
      '/Users/bangedorrunt/.local/share/toron/index.sqlite');
      print(c.execute('select max(version) from _sqlx_migrations').fetchone())"
