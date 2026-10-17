#!/usr/bin/env python3
"""Flip behavior rows in conformance/behavior-tests.tsv.

A row becomes `ported` only once a Rust test proves it, so this tool takes the
test path or function name beside the row and refuses to write a `ported` row
with an empty witness. It is the write half of `rows-port-remaining.py`.

Run from the repo root:

    python3 conformance/flip-row.py <test-path> <line>:<witness> [<line>:<witness> ...]
    python3 conformance/flip-row.py packages/effect/test/Cause.test.ts 722:no_such_element_error_shapes

`<witness>` goes in the `rust` column. For an integration test the port's
convention is the bare function name, which `ledger.rs` only requires to be
non-empty; use a `crates/...rs::fn` path when the row's own test is clearer.

Excusing a row (`--not-applicable "<reason>"`) writes the reason into the same
column, which `ledger.rs` also requires to be non-empty. Excusing is a
deliberate act with a stated cause, never a way to make a count go down.

A batch that is carried but waiting on the lease goes in a queue file and lands
in one call:

    python3 conformance/flip-row.py --from-queue conformance/queued-ledger-edits.tsv
    python3 conformance/flip-row.py --from-queue conformance/queued-ledger-edits.tsv --check

The ledger is one shared file under one lease, so an agent that has carried a
row but cannot take the lease stages it in the queue instead of holding the file
open between batches. `--check` reports what the queue would change and writes
nothing, which is the only way to verify a queue without the lease.
"""

from __future__ import annotations

import sys

LEDGER = "conformance/behavior-tests.tsv"

STATUSES = ("ported", "not-applicable", "skipped-upstream")


def load_queue(path: str) -> list[tuple[str, int, str, str]]:
    """Read `path<TAB>line<TAB>status<TAB>rust` rows, skipping `#` comments."""
    rows: list[tuple[str, int, str, str]] = []
    with open(path, encoding="utf-8") as handle:
        for raw in handle:
            if raw.startswith("#") or not raw.strip():
                continue
            fields = raw.rstrip("\n").split("\t")
            if len(fields) != 4:
                raise SystemExit(
                    f"{path}: expected 4 tab-separated fields, got {len(fields)}: {raw!r}"
                )
            test_path, line, status, witness = fields
            if status not in STATUSES:
                raise SystemExit(f"{path}:{line}: unknown status {status!r}")
            rows.append((test_path, int(line), status, witness))
    return rows


def main(argv: list[str]) -> int:
    args = list(argv[1:])
    if not args or args[0] in ("-h", "--help"):
        print(__doc__)
        return 0 if args else 2

    check = "--check" in args
    args = [arg for arg in args if arg != "--check"]

    wanted: dict[tuple[str, int], tuple[str, str]] = {}
    if args[0] == "--from-queue":
        if len(args) < 2:
            print("--from-queue needs a queue file", file=sys.stderr)
            return 2
        for test_path, line, status, witness in load_queue(args[1]):
            wanted[(test_path, line)] = (status, witness)
        scope = args[1]
    else:
        scope = args[0]
        rest = args[1:]
        if not rest:
            print(__doc__)
            return 2
        if rest[0] == "--not-applicable":
            if len(rest) < 2:
                print("--not-applicable needs a reason", file=sys.stderr)
                return 2
            wanted = {
                (scope, int(line)): ("not-applicable", rest[1]) for line in rest[2:]
            }
        else:
            for spec in rest:
                line, _, witness = spec.partition(":")
                if not line or not witness:
                    print(f"bad spec {spec!r}: expected <line>:<witness>", file=sys.stderr)
                    return 2
                wanted[(scope, int(line))] = ("ported", witness)

    lines = open(LEDGER, encoding="utf-8").read().splitlines(keepends=True)
    hits: dict[tuple[str, int], str] = {}

    for index, raw in enumerate(lines):
        fields = raw.rstrip("\n").split("\t")
        if len(fields) < 6:
            continue
        # The ledger's own header row, and any line whose third column is not
        # the row number the tool was given.
        if not fields[2].isdigit():
            continue
        key = (fields[1], int(fields[2]))
        if key not in wanted:
            continue
        status, witness = wanted[key]
        if not witness:
            print(f"{key}: refusing a {status} row with no witness", file=sys.stderr)
            return 1
        if fields[0] == status and fields[5] == witness:
            hits[key] = "already"
            continue
        if check:
            hits[key] = f"would be {status}"
            continue
        lines[index] = "\t".join([status] + fields[1:5] + [witness]) + "\n"
        hits[key] = status

    missing = set(wanted) - set(hits)
    if missing:
        for key in sorted(missing):
            print(f"{key}: no such behavior row", file=sys.stderr)
        return 1

    if not check:
        with open(LEDGER, "w", encoding="utf-8") as handle:
            handle.writelines(lines)

    changed = [key for key, value in hits.items() if value != "already"]
    verb = "would flip" if check else "flipped"
    print(
        f"{scope}: {len(changed)} rows {verb}, {len(wanted) - len(changed)} already set"
    )
    for key in sorted(changed):
        print(f"  :{key[1]} -> {hits[key]}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
