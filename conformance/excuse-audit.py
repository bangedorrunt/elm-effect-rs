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

Two more are rot that is worth failing on, because each is a fact the file
itself can answer:

  3. a reason that states a function count for a file it cites must state the
     current count, or state none at all.
  4. a reason that says `Module.name` is absent from a file it cites must not
     have `fn name` in that same file.

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
# A cited file plus the count of public functions the reason claims it has.
# The line suffix is optional: "absent from effect.rs (21 pub fns)" cites no line.
STALE_COUNT = re.compile(r"(crates/[A-Za-z0-9_./-]+\.rs)(?::\d+)?[^\n]*?\((\d+) pub fns")
FREE_COUNT = re.compile(r"(crates/[A-Za-z0-9_./-]+\.rs)(?::\d+)?[^\n]*?(\d+) free functions")
# Any cited file, with or without a line: an absence claim names the file it
# means, and the comma-separated spelling cites several at once.
ANY_CITATION = re.compile(r"crates/[A-Za-z0-9_./-]+\.rs")
# `effect.rs` on its own, which is how several reasons spell the file. Resolved
# by basename so the claim is still checked against the right file.
BARE_CITATION = re.compile(r"(?<![/\w])([a-z_][a-z0-9_]*\.rs)\b")


def resolve_bare(name: str) -> list[str]:
    found: list[str] = []
    for root, _, files in os.walk(CRATES):
        if "/target" in root or "/tests" in root:
            continue
        if name in files:
            found.append(os.path.join(root, name))
    return found
# Every `Module.member` in an absence sentence, so a list of them is checked
# in full rather than only its first entry.
ABSENT_NAME = re.compile(r"\b([A-Z][A-Za-z0-9]*)\.([A-Za-z0-9_]+)\b")
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


def stale_counts(rows: list[list[str]]) -> list[str]:
    """Reasons that state a function count for a file, checked against it.

    A count is the one part of an excuse that is trivially re-checkable, and
    the one part that rots on the first commit to that file: "(21 pub fns)"
    stayed in 48 reasons while the file grew to 59. A reason that needs no
    number should not carry one.
    """
    errors: list[str] = []
    actual: dict[str, int] = {}
    for row in rows:
        if row[0] == "unported":
            continue
        for pattern in (STALE_COUNT, FREE_COUNT):
            for match in pattern.finditer(row[5]):
                path, claimed = match.group(1), int(match.group(2))
                if path not in actual:
                    if not os.path.exists(path):
                        continue
                    with open(path, encoding="utf-8", errors="ignore") as handle:
                        actual[path] = sum(
                            1 for line in handle if re.match(r"\s*pub fn ", line)
                        )
                if actual[path] != claimed:
                    errors.append(
                        f"{row[1]}:{row[2]}: claims {claimed} in {path}, which has "
                        f"{actual[path]}"
                    )
    return errors


def absent_but_defined_here(rows: list[list[str]]) -> list[str]:
    """Absence claims contradicted by the very file the reason cites.

    Narrower than the workspace-wide name check below: when a reason says
    `Effect.flip` is absent from `effect.rs` and `fn flip` is in `effect.rs`,
    the claim is false, not a collision on a shared name.
    """
    errors: list[str] = []
    cache: dict[str, set[str]] = {}
    for row in rows:
        if row[0] == "unported":
            continue
        cited = set(ANY_CITATION.findall(row[5]))
        for bare in BARE_CITATION.findall(row[5]):
            cited.update(resolve_bare(bare))
        if not cited:
            continue
        names = ABSENT.findall(row[5])
        # "Effect.acquireUseRelease, Effect.delay and Effect.void are all absent"
        # names three, so every name in an absence sentence is checked, not just
        # the first the pattern happened to anchor on.
        # Split on `;` and on a period that ends a clause, never on the dot in
        # `Effect.delay`: that dot is part of the name being checked.
        for sentence in re.split(r";|\.\s", row[5]):
            marker = re.search(r"\babsent", sentence, re.I)
            if not marker:
                continue
            # Only the names before the word carry the claim: the tail of the
            # sentence often names an existing API for a different reason
            # ("... and Effect.ensuring takes Fn() rather than an Effect").
            names.extend(ABSENT_NAME.findall(sentence[: marker.start()]))
        for _module, member in names:
            name = snake(member)
            for path in cited:
                if path not in cache:
                    with open(path, encoding="utf-8", errors="ignore") as handle:
                        cache[path] = {
                            found.group(1)
                            for found in (
                                re.match(r"\s*(?:pub(?:\([^)]*\))? )?fn\s+([a-z_][a-z0-9_]*)", line)
                                for line in handle
                            )
                            if found
                        }
                if name in cache[path]:
                    errors.append(
                        f"{row[1]}:{row[2]}: claims `{name}` absent, but "
                        f"`fn {name}` is in {path}"
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

    counts = stale_counts(rows)
    failures.extend(counts)
    if counts:
        print(f"\nSTALE COUNTS: {len(counts)} reasons state a count the file contradicts")
        for error in counts[:20]:
            print(f"  {error}")

    defined = absent_but_defined_here(rows)
    failures.extend(defined)
    if defined:
        print(f"\nABSENT-BUT-DEFINED: {len(defined)} claims contradicted by the cited file")
        for error in defined[:20]:
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
