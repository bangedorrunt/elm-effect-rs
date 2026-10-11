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
"""

from __future__ import annotations

import sys

LEDGER = "conformance/behavior-tests.tsv"


def main(argv: list[str]) -> int:
    args = list(argv[1:])
    if len(args) < 2 or args[0] in ("-h", "--help"):
        print(__doc__)
        return 0 if args else 2

    test_path = args[0]
    rest = args[1:]

    not_applicable = None
    if rest[0] == "--not-applicable":
        if len(rest) < 2:
            print("--not-applicable needs a reason", file=sys.stderr)
            return 2
        not_applicable = rest[1]
        pairs = [(int(line), f"not-applicable: {not_applicable}") for line in rest[2:]]
    else:
        pairs = []
        for spec in rest:
            line, _, witness = spec.partition(":")
            if not line or not witness:
                print(f"bad spec {spec!r}: expected <line>:<witness>", file=sys.stderr)
                return 2
            pairs.append((int(line), witness))

    lines = open(LEDGER, encoding="utf-8").read().splitlines(keepends=True)
    wanted = {(test_path, line): witness for line, witness in pairs}
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
        status, witness = "ported", wanted[key]
        if witness.startswith("not-applicable: "):
            status, witness = "not-applicable", witness[len("not-applicable: ") :]
        if not witness:
            print(f"{key}: refusing a {status} row with no witness", file=sys.stderr)
            return 1
        if fields[0] == status and fields[5] == witness:
            hits[key] = "already"
            continue
        lines[index] = "\t".join([status] + fields[1:5] + [witness]) + "\n"
        fields[0] = status
        hits[key] = status

    missing = set(wanted) - set(hits)
    if missing:
        for key in sorted(missing):
            print(f"{key}: no such behavior row", file=sys.stderr)
        return 1

    with open(LEDGER, "w", encoding="utf-8") as handle:
        handle.writelines(lines)

    changed = [key for key, value in hits.items() if value != "already"]
    print(f"{test_path}: {len(changed)} rows flipped, {len(pairs) - len(changed)} already set")
    for key in sorted(changed):
        print(f"  :{key[1]} -> {hits[key]}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
