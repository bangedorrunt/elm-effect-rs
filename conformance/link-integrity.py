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

Run from the repo root:

    python3 conformance/link-integrity.py conformance/modules.toml
"""

from __future__ import annotations

import re
import sys

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

    for failure in failures:
        print(failure)
    if failures:
        print(f"\n{len(failures)} link failures")
        return 1

    print(f"ok: {len(MUST_LINK)} modules linked, {MUST_NOT_LINK_AI_MODEL} left unlinked")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
