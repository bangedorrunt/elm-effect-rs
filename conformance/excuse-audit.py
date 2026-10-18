#!/usr/bin/env python3
"""Audit the `not-applicable` reasons in conformance/behavior-tests.tsv.

An excuse is a claim: "this behavior cannot be represented in Rust". A claim
that cites a file and a line is checkable, and a claim that an API is absent
from a file is checkable against that file. Two of those checks gate:

  1. every data row has exactly six tab-separated columns. The old loader's
     `splitn(6)` absorbed an extra column silently, so 26 toIso rows carried a
     duplicated line number and every tool read `title = "it"`.
  2. every `crates/<path>.rs:<line>` a reason cites exists at that line. An
     excuse that points at a line that has moved is rot, and a reader who
     follows it lands nowhere.

The rest is a report, because name collisions are not proof: a claim that
`Channel.filterMapEffect` is missing can be true of the effectful variant while
`fn filter_map_effect` sits in the same file taking a synchronous predicate.

    python3 conformance/excuse-audit.py [--ledger PATH]

Exit 0 when both gates hold. Exit 1 names the row and the failed check.
"""

from __future__ import annotations

import collections
import os
import re
import sys

LEDGER = "conformance/behavior-tests.tsv"
CRATES = "crates"

CITATION = re.compile(r"(crates/[A-Za-z0-9_./-]+\.rs):(\d+)")
ABSENT = re.compile(r"\b([A-Z][A-Za-z0-9]*)\.([A-Za-z0-9_]+)\b[^.;]*?\babsent", re.I)
VAGUE = re.compile(
    r"depend(?:s)? on|not (?:fully )?(?:exposed|implemented|represented|available|ported)"
    r"|internals|parity with Effect",
    re.I,
)


def snake(name: str) -> str:
    return re.sub(r"(?<!^)(?=[A-Z])", "_", name).lower()


def load(path: str) -> tuple[list[list[str]], list[str]]:
    rows: list[list[str]] = []
    shape_errors: list[str] = []
    with open(path, encoding="utf-8") as handle:
        for number, raw in enumerate(handle, 1):
            if raw.startswith("#") or raw.startswith("status\t") or not raw.strip():
                continue
            fields = raw.rstrip("\n").split("\t")
            if len(fields) != 6:
                shape_errors.append(
                    f"line {number}: expected 6 tab-separated fields, got {len(fields)}"
                )
                continue
            rows.append(fields)
    return rows, shape_errors


def definitions() -> dict[str, str]:
    """Every `fn name` in the crates, mapped to its first definition site."""
    found: dict[str, str] = {}
    for root, _, files in os.walk(CRATES):
        if "/target" in root or "/tests" in root:
            continue
        for name in files:
            if not name.endswith(".rs"):
                continue
            path = os.path.join(root, name)
            with open(path, encoding="utf-8", errors="ignore") as handle:
                for number, line in enumerate(handle, 1):
                    match = re.match(r"\s*(?:pub(?:\([^)]*\))? )?fn\s+([a-z_][a-z0-9_]*)", line)
                    if match:
                        found.setdefault(match.group(1), f"{path}:{number}")
    return found


def citation_errors(rows: list[list[str]]) -> list[str]:
    errors: list[str] = []
    for row in rows:
        if row[0] == "unported":
            continue
        for match in CITATION.finditer(row[5]):
            path, line = match.group(1), int(match.group(2))
            if not os.path.exists(path):
                errors.append(f"{row[1]}:{row[2]}: cites missing file {path}")
                continue
            with open(path, encoding="utf-8", errors="ignore") as handle:
                total = sum(1 for _ in handle)
            if line > total:
                errors.append(
                    f"{row[1]}:{row[2]}: cites {path}:{line}, the file has {total} lines"
                )
    return errors


def main(argv: list[str]) -> int:
    path = LEDGER
    if "--ledger" in argv:
        path = argv[argv.index("--ledger") + 1]

    rows, shape_errors = load(path)
    statuses = collections.Counter(row[0] for row in rows)

    print(f"{path}: {len(rows)} rows")
    for status, count in sorted(statuses.items(), key=lambda item: -item[1]):
        print(f"  {status:16s} {count:5d}")

    failures = list(shape_errors)
    if shape_errors:
        print(f"\nSHAPE: {len(shape_errors)} malformed rows")
        for error in shape_errors[:20]:
            print(f"  {error}")

    rot = citation_errors(rows)
    failures.extend(rot)
    if rot:
        print(f"\nCITATIONS: {len(rot)} reasons point at a line that is not there")
        for error in rot[:20]:
            print(f"  {error}")

    excuses = [row for row in rows if row[0] == "not-applicable"]
    print(f"\nexcused rows: {len(excuses)}")

    blanket = collections.Counter(row[5] for row in excuses)
    repeated = [(reason, count) for reason, count in blanket.most_common() if count >= 10]
    if repeated:
        print(f"\nBLANKET REASONS ({len(repeated)} texts cover "
              f"{sum(count for _, count in repeated)} rows)")
        for reason, count in repeated[:12]:
            files = {row[1] for row in excuses if row[5] == reason}
            print(f"  {count:4d}  {reason[:96]}")
            if len(files) > 1:
                print(f"        spans {len(files)} test files")

    found = definitions()
    absent = collections.Counter()
    for row in excuses:
        for match in ABSENT.finditer(row[5]):
            absent[f"{match.group(1)}.{match.group(2)}"] += 1
    resolved = {
        name: found[snake(name.split(".")[1])]
        for name in absent
        if snake(name.split(".")[1]) in found
    }
    if resolved:
        print(f"\nCLAIMED-ABSENT APIS THAT HAVE A DEFINITION ({len(resolved)} names, "
              f"{sum(absent[name] for name in resolved)} rows)")
        print("  a name resolving is not proof the claim is false: the cited variant may differ")
        for name, where in sorted(resolved.items()):
            print(f"  {absent[name]:4d}  {name:34s} {where}")

    vague = [row for row in excuses if VAGUE.search(row[5])]
    if vague:
        by_module = collections.Counter(row[1] for row in vague)
        print(f"\nVAGUE REASONS: {len(vague)} rows name no API or line")
        for module, count in by_module.most_common(10):
            print(f"  {count:4d}  {module.replace('packages/effect/test/', '')}")

    if failures:
        print(f"\nFAIL: {len(failures)} checks failed")
        return 1
    print("\nOK: shape and citation gates hold")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
