#!/usr/bin/env python3
"""Assert the module-to-test links in `conformance/modules.toml`.

`inventory.sh` links a src module to its test two ways: by the path relative
to `packages/effect/src/` and `packages/effect/test/`, which only match when
the module and its test share a subdirectory, and by basename as a fallback.
Both have bitten. The key derivation missed `src/Schema.ts` (tested from
`test/schema/Schema.test.ts`) and `src/testing/TestClock.ts` (tested from
`test/TestClock.test.ts`); the name filter ran `find -name '*.test.ts'` and
so never saw `FooTest.ts`. Modules whose test was invisible read
`tests = []`, which `port_like_unmatched()` counts forever.

The basename fallback has one trap worth pinning: `test/ai/Model.test.ts`
belongs to `src/ai/Model.ts`, and a fallback that ignores claims hands it to
`src/schema/Model.ts` too. So this checks the positive links AND that `Model`
stays unlinked rather than mislinked.

The second assertion is about the files no path rule can reach. A test file
whose rows are linked to no module row moves no module's status, so the
phase-13 gate cannot count its work: 88 such files held 1983 open rows. The
third link route in `inventory.sh` (a test that sits in a directory named
after a module it imports) is the only derivation the pin's paths support, and
it covers 8 of them. Every other open-row file is recorded in
`conformance/unowned-tests.tsv` with its reason, and this fails if one is in
neither set, if a recorded file is linked after all, or if a recorded file
stopped having open rows. The route never chases a near-miss name:
`test/cli/Arguments.test.ts` (plural) is not `src/cli/Argument.ts` (singular)
by any derivation, only by a rename someone approved.

Run from the repo root:

    python3 conformance/link-integrity.py conformance/modules.toml
"""

from __future__ import annotations

import re
import sys

LEDGER = "conformance/behavior-tests.tsv"
UNOWNED = "conformance/unowned-tests.tsv"

# Modules whose test exists at the pin but that the link derivation missed.
# Each must now carry a non-empty `tests` field.
MUST_LINK = (
    "packages/effect/src/Schema.ts",
    "packages/effect/src/SchemaAST.ts",
    "packages/effect/src/SchemaGetter.ts",
    "packages/effect/src/SchemaIssue.ts",
    "packages/effect/src/SchemaParser.ts",
    "packages/effect/src/SchemaRepresentation.ts",
    "packages/effect/src/SchemaTransformation.ts",
    "packages/effect/src/Arbitrary.ts",
    "packages/effect/src/ai/McpProtocol.ts",
    "packages/effect/src/ai/McpServer.ts",
    "packages/effect/src/http-api/HttpApi.ts",
    "packages/effect/src/http-api/HttpApiBuilder.ts",
    "packages/effect/src/http-api/HttpApiClient.ts",
    "packages/effect/src/http-api/HttpApiEndpoint.ts",
    "packages/effect/src/http-api/HttpApiSchema.ts",
    "packages/effect/src/http-api/HttpApiSecurity.ts",
    "packages/effect/src/http-api/OpenApi.ts",
    "packages/effect/src/reactivity/AtomRef.ts",
    "packages/effect/src/sql/Migrator.ts",
    "packages/effect/src/testing/TestClock.ts",
    "packages/effect/src/process/ChildProcessSpawner.ts",
    "packages/effect/src/persistence/RateLimiter.ts",
)

# `test/ai/Model.test.ts` is `src/ai/Model.ts`'s. If `src/schema/Model.ts`
# ever links it, the fallback is stealing and must be fixed, not celebrated.
MUST_NOT_LINK_AI_MODEL = "packages/effect/src/schema/Model.ts"
AI_MODEL_TEST = "packages/effect/test/ai/Model.test.ts"


def parse(text: str) -> dict[str, list[str]]:
    modules: dict[str, list[str]] = {}
    for block in re.findall(r"(?s)\[\[module\]\]\n(.*?)(?=\n\[\[module\]\]|\Z)", text):
        path = re.search(r'path = "([^"]+)"', block)
        tests = re.search(r"tests = \[(.*?)\]", block, re.S)
        if not path:
            continue
        modules[path.group(1)] = [
            t.strip('" ') for t in (tests.group(1).split(",") if tests else []) if t.strip('" ')
        ]
    return modules


def open_row_files(path: str) -> set[str]:
    """Every test file in the behavior ledger that still owes a port."""
    files: set[str] = set()
    with open(path, encoding="utf-8") as handle:
        for line in handle:
            if line.startswith("#") or line.startswith("status\t"):
                continue
            fields = line.rstrip("\n").split("\t")
            if len(fields) >= 6 and fields[0] == "unported":
                files.add(fields[1])
    return files


def recorded_unowned(path: str) -> dict[str, str]:
    """The hand-kept list of open-row test files no path rule can link."""
    if not path:
        return {}
    try:
        handle = open(path, encoding="utf-8")
    except FileNotFoundError:
        return {}
    with handle:
        out: dict[str, str] = {}
        for line in handle:
            if line.startswith("#") or not line.strip():
                continue
            fields = line.rstrip("\n").split("\t")
            if len(fields) >= 2 and fields[0]:
                out[fields[0]] = fields[1]
        return out


def main(argv: list[str]) -> int:
    if len(argv) != 2:
        print(f"usage: {argv[0]} conformance/modules.toml", file=sys.stderr)
        return 2

    with open(argv[1], encoding="utf-8") as handle:
        modules = parse(handle.read())

    failures: list[str] = []

    for path in MUST_LINK:
        if path not in modules:
            failures.append(f"{path}: no module row")
        elif not modules[path]:
            failures.append(f"{path}: tests = [] (its test exists at the pin)")

    if MUST_NOT_LINK_AI_MODEL in modules and AI_MODEL_TEST in modules[MUST_NOT_LINK_AI_MODEL]:
        failures.append(
            f"{MUST_NOT_LINK_AI_MODEL}: links {AI_MODEL_TEST}, "
            "which belongs to packages/effect/src/ai/Model.ts"
        )

    linked: dict[str, list[str]] = {}
    for module, tests in modules.items():
        for test in tests:
            linked.setdefault(test, []).append(module)

    open_files = open_row_files(LEDGER)
    recorded = recorded_unowned(UNOWNED)

    for path in sorted(open_files - set(linked) - set(recorded)):
        failures.append(
            f"{path}: open behavior rows, linked to no module row and not recorded unowned"
        )
    for path in sorted(set(recorded) - open_files):
        failures.append(f"{path}: recorded unowned but carries no open rows (stale entry)")
    for path in sorted(set(recorded) & set(linked)):
        failures.append(
            f"{path}: recorded unowned but linked by {', '.join(sorted(linked[path]))}"
        )

    for failure in failures:
        print(failure)
    if failures:
        print(f"\n{len(failures)} link failures")
        return 1

    print(
        f"ok: {len(MUST_LINK)} modules linked, {MUST_NOT_LINK_AI_MODEL} left unlinked, "
        f"{len(open_files)} open-row test files linked or recorded unowned "
        f"({len(set(open_files) & set(linked))} linked, {len(recorded)} unowned)"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
