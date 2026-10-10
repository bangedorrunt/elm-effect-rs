#!/usr/bin/env python3
"""Report the behavior rows a port bead still owes for one test file.

`port-module.md` closes a bead only when every behavior row the task named is
`ported` and the destination crate is green. The row state lives in
`conformance/behavior-tests.tsv`, so this is the half of a bead's VERIFY fence
that speaks about the ledger; the other half is the `cargo test -p` the bead
names alongside it.

Run from the repo root:

    python3 conformance/rows-port-remaining.py packages/effect/test/Stream.test.ts

Exits 0 with a count when every row is `ported` or `not-applicable`. Exits 1
and lists what is left when any row is still `unported`, so the failure names
the work rather than just failing.
"""

from __future__ import annotations

import sys

PORTED = ("ported", "not-applicable", "skipped-upstream")


def main(argv: list[str]) -> int:
    if len(argv) != 2:
        print(f"usage: {argv[0]} <test path as it appears in behavior-tests.tsv>", file=sys.stderr)
        return 2

    wanted = argv[1]
    open_rows: list[tuple[str, int, str]] = []
    total = 0

    with open("conformance/behavior-tests.tsv", encoding="utf-8") as handle:
        for line in handle:
            if line.startswith("#") or line.startswith("status\t"):
                continue
            fields = line.rstrip("\n").split("\t")
            if len(fields) < 6 or fields[1] != wanted:
                continue
            total += 1
            if fields[0] == "unported":
                open_rows.append((fields[2], fields[3], fields[4]))

    if total == 0:
        print(f"{wanted}: no behavior rows in the ledger", file=sys.stderr)
        return 1

    if open_rows:
        print(f"{wanted}: {len(open_rows)} of {total} rows still unported")
        for line, kind, title in open_rows:
            print(f"  :{line} ({kind}) {title}")
        return 1

    print(f"{wanted}: all {total} rows ported or excused")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
