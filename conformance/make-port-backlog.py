#!/usr/bin/env python3
"""Create the porting backlog from conformance/behavior-tests.tsv.

One bead per Effect test file that already has ported rows, i.e. whose Rust
surface exists, so each bead is incremental work rather than a subsystem
build. `port-module.md` sets the granularity at one Effect module; the repo's
own precedent (elm-effect-rs-g3a) is one bead per row cluster from one test
file, which is what this creates.

Rows are NOT copied into the description. They move as the port progresses, so
a description that lists them is stale the moment the first row flips. Instead
each bead names `python3 conformance/rows-port-remaining.py <path>`, which
reads the live ledger. The bead's VERIFY fence is the crate test plus that
command, so a bead cannot close while a named row is still `unported`.

Run from the repo root.
"""

from __future__ import annotations

import collections
import json
import os
import re
import subprocess
import sys

MODULES = "conformance/modules.toml"
LEDGER = "conformance/behavior-tests.tsv"
OPEN = ("unported",)
DONE = ("ported", "not-applicable", "skipped-upstream")

# Bands by open-row count. The loop drains `--wave-label wave-N` and holds a
# wave while a lower one is open, so the biggest clusters land first and the
# conformance ledger is not contended by 86 agents at once.
WAVES = ((60, 1), (20, 2), (6, 3), (0, 4))

DESCRIPTION = """Port {name}'s {n} open behavior rows from the Effect v4.0.0 pin.

The Rust surface for this file already exists ({done} of {total} behavior rows
are `ported`), so this is incremental: carry the remaining rows, do not rebuild
what passes.

## What to read

- The TypeScript at the pin: `~/.cache/effect-4.0.0/packages/effect/test{path}`
  (and the module under `packages/effect/src/` that it exercises).
- The live row list, which is the spec:

      python3 conformance/rows-port-remaining.py {path}

## Rules from the elm-effect skill's port-module reference

- Carry a row only when its behavior is representable in Rust. A carried `it.each`
  row runs every tuple in its table. A test that only asserts the function exists
  does not port the row.
- A row we will not carry is `not-applicable` with a reason in the `rust` column.
  Do not mass-excuse: the ledger's own history calls a run that excused all 6940
  open rows "parity theater", and `matched` is derived, so an excused row cannot
  quietly buy parity.
- `Equal` for domain values treats NaN as equal; `f64`'s `==` stays IEEE.
- Waits go through `Clock`. A direct `tokio::time::sleep` inside the module is a bug.
- Set a row to `ported` only after its Rust test was red and then green, and put
  that test's path in the `rust` column. A `ported` row with an empty `rust`
  column fails `cargo test -p elm-effect-conformance --test ledger`.
- Flip the ledger under a lease: `toron reserve acquire`, edit
  `conformance/behavior-tests.tsv`, commit, release. Never hold the lease between
  batches.

## Done

`cargo test -p {crate}` passes and `conformance/rows-port-remaining.py` reports
zero open rows for this file.
"""


def wave_for(n: int) -> int:
    for threshold, wave in WAVES:
        if n >= threshold:
            return wave
    return WAVES[-1][1]


def witness_crates(names: dict[str, list[str]]) -> dict[str, collections.Counter]:
    """Crate votes per test path, from where its ported rows are witnessed.

    A ported row's `rust` column names a test function (2940 of 2942 rows), so
the crate is whichever crate defines that function. This is the strongest
evidence of where a file's Rust surface lives, and the only signal that works
when the module row links no test at all (class `port` then says
"elm-effect" for rows whose witnesses all live in elm-effect-schema, and the
fence passes no matter what the port does).
    """
    defers: dict[str, set[str]] = collections.defaultdict(set)
    for root, dirs, files in os.walk("crates"):
        if "/target" in root:
            continue
        for name in files:
            if not name.endswith(".rs"):
                continue
            p = os.path.join(root, name)
            try:
                text = open(p, encoding="utf-8", errors="ignore").read()
            except OSError:
                continue
            crate = p.split("/")[1]
            for m in re.finditer(r"\bfn\s+([a-z_][a-z0-9_]*)\s*\(", text):
                defers[m.group(1)].add(crate)
    votes: dict[str, collections.Counter] = collections.defaultdict(collections.Counter)
    for path, fns in names.items():
        for fn in fns:
            for crate in defers.get(fn, ()):
                votes[path][crate] += 1
    return votes


def crate_for(
    path: str,
    test2class: dict[str, str],
    rust_crate: dict[str, str],
    witness_vote: dict[str, collections.Counter],
) -> str:
    # Witness first: a module row's class says what KIND of module it is, not
    # which crate carries its port. elm-effect-schema's rows are class `port`,
    # and fencing them against elm-effect runs tests that do not exist there.
    if witness_vote.get(path):
        return max(witness_vote[path], key=witness_vote[path].get)
    cls = test2class.get(path)
    if cls == "port":
        return "elm-effect"
    if cls in ("unstable-port", "experimental-port"):
        return "elm-effect-unstable"
    if rust_crate.get(path):
        return max(rust_crate[path], key=rust_crate[path].get)
    return "elm-effect"


def main() -> int:
    mods = re.findall(
        r"(?s)\[\[module\]\]\n(.*?)(?=\n\[\[module\]\]|\Z)", open(MODULES, encoding="utf-8").read()
    )
    test2class: dict[str, str] = {}
    for block in mods:
        cls = re.search(r'class = "([^"]+)"', block)
        tests = re.search(r"tests = \[(.*?)\]", block, re.S)
        if not cls:
            continue
        for test in [t.strip('" ') for t in (tests.group(1).split(",") if tests else []) if t.strip('" ')]:
            test2class[test] = cls.group(1)

    rows: dict[str, collections.Counter] = collections.defaultdict(collections.Counter)
    rust_crate: dict[str, str] = collections.defaultdict(collections.Counter)
    witness_name: dict[str, list[str]] = collections.defaultdict(list)
    with open(LEDGER, encoding="utf-8") as handle:
        for line in handle:
            if line.startswith("#") or line.startswith("status\t"):
                continue
            f = line.rstrip("\n").split("\t")
            if len(f) < 6 or f[0] not in OPEN + DONE:
                continue
            rows[f[1]][f[0]] += 1
            if f[0] == "ported" and f[5] and not f[5].startswith("crates/"):
                witness_name[f[1]].append(f[5])
            # A ported row's `rust` column names the test file, so the crate is
            # whatever crate that file lives in. Used only when the module row
            # does not link the test at all. The column's convention is a bare
            # test function name (2940 of 2942 rows), which carries no path and
            # therefore no crate; only a `crates/...` path does, so anything
            # else is skipped rather than bucketed under a placeholder that
            # would later win the max below.
            if f[0] == "ported" and f[5].startswith("crates/"):
                rust_crate[f[1]][f[5].split("/")[1]] += 1

    witness_vote = witness_crates(witness_name)
    backlog = []
    for path, counts in rows.items():
        n = counts["unported"]
        if n == 0 or counts["ported"] == 0:
            continue
        crate = crate_for(path, test2class, rust_crate, witness_vote)
        backlog.append((path, n, counts["ported"], sum(counts.values()), crate))

    backlog.sort(key=lambda item: (-item[1], item[0]))
    os.makedirs("/tmp/port-beads", exist_ok=True)

    by_wave: dict[int, int] = collections.Counter()
    for path, n, done, total, crate in backlog:
        wave = wave_for(n)
        by_wave[wave] += 1
        name = path.rsplit("/", 1)[-1].replace(".test.ts", "").replace(".test-utils.ts", "")
        slug = re.sub(r"[^a-z0-9]+", "-", name.lower()).strip("-")[:40]
        body = DESCRIPTION.format(
            name=name, n=n, done=done, total=total, path=path, crate=crate
        )
        desc_path = f"/tmp/port-beads/{slug}.md"
        with open(desc_path, "w", encoding="utf-8") as handle:
            handle.write(body)

        verify = (
            f'sh -c "cargo test -p {crate} && '
            f'python3 conformance/rows-port-remaining.py {path}"'
        )
        principle = (
            f"behavior-parity-is-witnessed — {n} unported rows in {path} each earn a "
            "Rust test that was red then green, so the row's ported status names a "
            "real test path instead of an assertion that the function exists"
        )
        title = f"Port {name}'s {n} open behavior rows"

        result = subprocess.run(
            [
                "br", "create", title,
                "--slug", f"port-{slug}",
                "-t", "task", "-p", "2",
                "--description-file", desc_path,
                "--verify", verify,
                "--principle", principle,
                "--deliverable", "diff",
                "--wave", str(wave),
                "--labels", f"wave-{wave}",
                "--json",
            ],
            capture_output=True,
            text=True,
        )
        if result.returncode != 0:
            print(f"FAILED {path}: {result.stderr.strip()[:200]}")
            continue
        try:
            print(f"wave{wave}  {json.loads(result.stdout)['created']['id']}  {n:4d} rows  {path}")
        except Exception:
            print(f"wave{wave}  ??? {n} rows  {path}")

    print()
    print("beads per wave:", dict(sorted(by_wave.items())))
    print("total open rows covered:", sum(item[1] for item in backlog))
    return 0


def self_check() -> int:
    """Pin crate inference against the cases that exposed the old bug.

    Every expectation was measured on this tree: these files' ported rows are
    witnessed in the named crate, while their module rows are classed `port`
    (or link nothing), which is why the class-first rule fenced them against
    elm-effect, where none of those tests exist.
    """
    names: dict[str, list[str]] = collections.defaultdict(list)
    with open(LEDGER, encoding="utf-8") as handle:
        for line in handle:
            if line.startswith("#") or line.startswith("status\t"):
                continue
            f = line.rstrip("\n").split("\t")
            if len(f) >= 6 and f[0] == "ported" and f[5] and not f[5].startswith("crates/"):
                names[f[1]].append(f[5])
    votes = witness_crates(names)
    expected = {
        "packages/effect/test/schema/Schema.test.ts": "elm-effect-schema",
        "packages/effect/test/schema/SchemaParser.test.ts": "elm-effect-schema",
        "packages/effect/test/schema/toCodec.test.ts": "elm-effect-schema",
        "packages/effect/test/encoding/SchemaBinary.test.ts": "elm-effect-unstable",
        "packages/effect/test/schema/Graph.test.ts": "elm-effect",
    }
    bad = 0
    for path, want in expected.items():
        got = crate_for(path, {}, {}, votes)
        if got == want:
            print(f"ok   {path} -> {got}")
        else:
            print(f"FAIL {path}: got {got}, want {want}")
            bad += 1
    if not votes:
        print("FAIL no witness votes at all (wrong cwd, or ledger empty?)")
        bad += 1
    return 1 if bad else 0


if __name__ == "__main__":
    if "--self-check" in sys.argv:
        sys.exit(self_check())
    sys.exit(main())
