#!/usr/bin/env python3
"""Audit the `ported` and `not-applicable` claims in conformance/behavior-tests.tsv.

`ledger.rs` checks the shape of this file. Shape is not truth. A `ported` row
says "a Rust test proves this behavior", and the `rust` column names the
witness. Two claims hide behind that word:

  * the witness exists (a test fn, a test file stem, or `path::fn`, where
    the path may be a `tests/` file or a `src/` file with an inline
    `#[cfg(test)]` module), and
  * the witness exercises the port (its body reaches a symbol the crates
    define), not the Rust standard library.

The second is the falsifier. `array_std_substrate` calls `vec![]`, `Vec::from`
and `Vec::new` and asserts their behavior; it passes with zero elm-effect code
in the build. When 112 Array rows are marked `ported` against that test, the
status is bought from `std`, not from the port.

A `not-applicable` row says "this behavior cannot be represented in Rust". The
citation half is gated by `excuse-audit.py`. This script gates the claim half:

  * a reason that says `X.foo ... absent` must not name an API that exists, and
  * the reason must name something specific (an API, a cited line, or a closed
    domain), not only "depends on internals".

    python3 conformance/witness-audit.py [--ledger PATH] [--artifact PATH]

Exit 0 when the mechanical gates hold. Exit 1 naming every failing row.
"""

from __future__ import annotations

import argparse
import collections
import os
import re
import sys

LEDGER = "conformance/behavior-tests.tsv"
ARTIFACT = "conformance/witness-audit.tsv"
CRATES = "crates"

TEST_FN = re.compile(
    r"(?m)^[ \t]*(?:pub[ \t]+)?(?:async[ \t]+)?fn[ \t]+([a-z0-9_]+)[ \t]*\("
)
ATTR = re.compile(r"(?m)^[ \t]*#\[(test|tokio::test|ignore[^\]]*)\]")
IDENT = re.compile(r"[A-Za-z_][A-Za-z0-9_]*")
LOCAL_FN = re.compile(
    r"(?m)^[ \t]*(?:pub[ \t]+)?fn[ \t]+([a-z0-9_]+)[ \t]*\("
)

# Names the Rust standard library (and serde/tokio) own. A body that only
# names these tests the substrate.
STD = {
    "Vec", "VecDeque", "Option", "Result", "String", "str", "Box", "Rc", "Arc",
    "Cell", "RefCell", "HashMap", "HashSet", "BTreeMap", "BTreeSet", "Cow",
    "Duration", "Instant", "SystemTime", "Path", "PathBuf", "OsString",
    "OsStr", "Ordering", "Iterator", "IntoIterator", "Default", "Clone",
    "Copy", "Debug", "Display", "Error", "Formatter", "PhantomData",
    "PhantomPinned", "Some", "None", "Ok", "Err", "Self", "assert",
    "assert_eq", "assert_ne", "panic", "todo", "unimplemented", "matches",
    "vec", "format", "println", "eprintln", "write", "writeln", "min", "max",
    "sum", "map", "and_then", "or_else", "unwrap", "expect", "collect", "iter",
    "into_iter", "filter", "fold", "zip", "enumerate", "to_string", "as_ref",
    "as_mut", "as_str", "len", "is_empty", "push", "pop", "insert", "remove",
    "get", "contains", "contains_key", "insert_at", "drop", "take", "clone",
    "eq", "cmp", "hash", "now", "sleep", "yield_now", "json",
}

# `Effect.onInterrupt is absent`, `Channel.never ... absent`, `Schema.Option
# does not exist`: the claim is that the API is not there, so a definition is
# a falsifier.
# Methods the standard library owns. A witness that only calls these is
# exercising Vec/Slice/Iterator, not the port, even when the port happens to
# define a same-named method. Keep this list growing from evidence: every
# entry here was a false positive in an earlier pass.
STD_METHODS = {
    "partition", "flatten", "reverse", "sort", "sort_by", "sort_unstable",
    "dedup", "dedup_by", "contains", "sum", "extend", "insert", "clone",
    "len", "is_empty", "iter", "iter_mut", "into_iter", "collect", "map",
    "filter", "filter_map", "flat_map", "fold", "take", "skip", "zip",
    "enumerate", "rev", "next", "position", "rposition", "any", "all",
    "find", "find_map", "count", "max", "min", "max_by", "min_by", "split",
    "splitn", "split_off", "starts_with", "ends_with", "trim", "parse",
    "chars", "bytes", "lines", "join", "repeat", "replace", "as_bytes",
    "as_slice", "as_str", "as_mut_slice", "first", "last", "push", "pop",
    "remove", "retain", "drain", "clear", "resize", "truncate", "capacity",
    "swap", "get", "get_mut", "unwrap", "expect", "unwrap_or", "unwrap_or_else",
    "ok_or", "and_then", "or_else", "map_or", "map_err", "is_ok", "is_err",
    "is_some", "is_none", "is_nan", "is_finite", "is_infinite", "to_bits",
    "from_bits", "abs", "signum", "powi", "sqrt", "floor", "ceil", "round",
    "product", "contains_key", "entry", "keys", "values", "to_vec", "to_owned",
    "to_string", "to_lowercase", "to_uppercase", "hash", "fmt", "write_str",
}

ABSENT = re.compile(
    r"\b([A-Z][A-Za-z0-9]*)\.([A-Za-z0-9_]+)\b[^.;]{0,80}?"
    r"\b(?:absent|does not exist|do not exist|is missing|are missing)\b",
    re.I,
)
FILE_REF = re.compile(r"crates/[A-Za-z0-9_./-]+\.rs")
SPECIFIC = re.compile(
    r"crates/[A-Za-z0-9_./-]+\.rs:\d+|\b[A-Z][A-Za-z0-9]*\.[a-zA-Z][A-Za-z0-9_]*\b"
    r"|no [a-z]+ domain|cannot be represented|unrepresentable|by construction"
    r"|not representable|would need|requires a [a-z]+ (?:type|domain)"
    r"|inexpressible"
)


def read(path: str) -> str:
    with open(path, encoding="utf-8", errors="ignore") as handle:
        return handle.read()


def snake(name: str) -> str:
    return re.sub(r"(?<!^)(?=[A-Z])", "_", name).lower()


def module_files() -> dict[str, list[str]]:
    """snake(stem) -> every src file with that stem, across the crates.

    `Stream.ignore` is a claim about stream.rs, so the check has to read
    stream.rs first: a `ignore` helper in another crate does not make the
    claim false.
    """
    out: dict[str, list[str]] = collections.defaultdict(list)
    for root, dirs, files in os.walk(CRATES):
        dirs[:] = [d for d in dirs if d not in ("target", "tests")]
        for name in files:
            if name.endswith(".rs"):
                out[snake(name[:-3])].append(os.path.join(root, name))
    return out


def port_symbols() -> tuple[set[str], set[str]]:
    """(type names, fn names) the crates define.

    Split because a bare lowercase word is usually a std method (`map`, `get`)
    while a capitalised one is the port's own type (`Decimal`, `Schema`).
    """
    types: set[str] = set()
    fns: set[str] = set()
    for root, dirs, files in os.walk(CRATES):
        dirs[:] = [d for d in dirs if d not in ("target", "tests")]
        if f"{os.sep}tests" in root:
            continue
        for name in files:
            if not name.endswith(".rs"):
                continue
            text = read(os.path.join(root, name))
            for match in re.finditer(
                r"(?m)^[ \t]*pub(?:\([^)]*\))?[ \t]+(mod|struct|enum|trait|type|const|static|fn)[ \t]+([A-Za-z_][A-Za-z0-9_]*)",
                text,
            ):
                kind, symbol = match.group(1), match.group(2)
                (fns if kind == "fn" else types).add(symbol)
            # Methods inside `impl` blocks are the port's public surface too:
            # `hash_value` on a value is the port's Hash, not a std method.
            for match in re.finditer(
                r"(?m)^[ \t]+(?:pub(?:\([^)]*\))?[ \t]+)?(?:async[ \t]+)?fn[ \t]+([A-Za-z_][A-Za-z0-9_]*)",
                text,
            ):
                fns.add(match.group(1))
            stem = name[:-3]
            if stem not in ("lib", "mod", "main"):
                types.add(stem)
    return types - STD, fns - STD


def masked(text: str) -> str:
    """Blanks strings, chars and comments so brace counting sees only code.

    A `{` inside a literal or a comment leaves the depth wrong, and a body
    that runs on into its neighbours is attributed to the wrong test.
    """
    out = list(text)
    i, n = 0, len(text)
    while i < n:
        char = text[i]
        if char == "/" and i + 1 < n and text[i + 1] == "/":
            while i < n and text[i] != "\n":
                out[i] = " "
                i += 1
        elif char == "/" and i + 1 < n and text[i + 1] == "*":
            depth = 0
            while i < n:
                if text.startswith("/*", i):
                    depth += 1
                    out[i] = out[i + 1] = " "
                    i += 2
                    continue
                if text.startswith("*/", i):
                    depth -= 1
                    out[i] = out[i + 1] = " "
                    i += 2
                    if depth == 0:
                        break
                    continue
                out[i] = " "
                i += 1
        elif char == '"':
            out[i] = " "
            i += 1
            while i < n:
                if text[i] == "\\":
                    out[i] = " "
                    if i + 1 < n:
                        out[i + 1] = " "
                    i += 2
                    continue
                if text[i] == '"':
                    out[i] = " "
                    i += 1
                    break
                out[i] = " "
                i += 1
        elif char == "'":
            # A char literal closes within a few bytes; a lifetime does not.
            close = text.find("'", i + 1, i + 8)
            if close > i and "\n" not in text[i : close + 1]:
                for j in range(i, close + 1):
                    out[j] = " "
                i = close + 1
            else:
                i += 1
        else:
            i += 1
    return "".join(out)


def body_of(text: str, open_brace: int) -> str:
    depth = 0
    code = masked(text)
    for index in range(open_brace, len(code)):
        char = code[index]
        if char == "{":
            depth += 1
        elif char == "}":
            depth -= 1
            if depth == 0:
                return text[open_brace : index + 1]
    return text[open_brace:]


def fn_bodies(path: str) -> dict[str, str]:
    """Every fn body in one file, keyed by name. First definition wins."""
    text = read(path)
    out: dict[str, str] = {}
    for match in LOCAL_FN.finditer(text):
        brace = text.find("{", match.end())
        if brace >= 0:
            out.setdefault(match.group(1), body_of(text, brace))
    return out


def split_witness(token: str) -> tuple[str, str]:
    """A `path::fn` or `path::module::fn` cell as (path, fn).

    The module segments between the file and the test are noise: a src-inline
    `mod tests` nests its tests one level down, so `file::tests::row` must
    resolve like `file::row`.
    """
    parts = token.split("::")
    return parts[0], parts[-1]


def test_fns(path: str) -> set[str]:
    """Names carrying #[test]. An audit witness must be a real test."""
    text = read(path)
    attrs = [(m.start(), m.group(1)) for m in ATTR.finditer(text)]
    out: set[str] = set()
    for match in TEST_FN.finditer(text):
        preceding = [a for a in attrs if a[0] < match.start()][-1:]
        if not preceding:
            continue
        if preceding[0][1] == "test" or "ignore" in preceding[0][1]:
            out.add(match.group(1))
    return out


def references(
    body: str, types: set[str], fns: set[str], helpers: dict[str, str], depth: int = 3
) -> set[str]:
    """Port symbols this body (or a file-local helper it calls) names.

    Two routes count. A port type token (`Decimal`) or a qualified path
    (`array::head`) is direct evidence. A call to a file-local helper
    (`dec("1")` -> `Decimal::from_string`) is indirect, so helpers expand;
    without that every wrapper-based test looks like a substrate test.
    """
    hits: set[str] = set()
    for match in re.finditer(r"([A-Za-z_][A-Za-z0-9_]*)[ ]*::", body):
        if match.group(1) in types or match.group(1) in fns:
            hits.add(match.group(1))
    # Only the qualifier proves the call: `Vec::from` and `Result::is_ok` name
    # std types even when the port defines a `from` or an `is_ok` of its own.
    for token in set(IDENT.findall(body)):
        if token in types:
            hits.add(token)
    # A bare call to a fn the crates define, or a method call on a type they
    # own, is direct use of the port even when the fn lives in another module.
    for match in re.finditer(r"(?<![.\w])([a-z_][a-z0-9_]*)[ ]*\(", body):
        if match.group(1) in fns:
            hits.add(match.group(1))
    for match in re.finditer(r"\.([a-z_][a-z0-9_]*)[ ]*\(", body):
        name = match.group(1)
        if name in fns and name not in STD_METHODS:
            hits.add(name)
    if depth > 0:
        for call in set(re.findall(r"\b([a-z_][a-z0-9_]*)[ ]*\(", body)):
            helper = helpers.get(call)
            if helper is None:
                continue
            hits |= references(helper, types, fns, helpers, depth - 1)
    return hits


def load_ledger(path: str) -> tuple[list[list[str]], list[str]]:
    rows: list[list[str]] = []
    errors: list[str] = []
    for number, raw in enumerate(read(path).splitlines(), 1):
        if raw.startswith("#") or raw.startswith("status\t") or not raw.strip():
            continue
        fields = raw.split("\t")
        if len(fields) != 6:
            errors.append(f"{path}:{number}: {len(fields)} fields, not 6")
            continue
        rows.append(fields)
    return rows, errors


class Witnesses:
    """Index of the Rust side: tests per file, helpers per file, stems."""

    def __init__(self) -> None:
        self.tests: dict[str, dict[str, str]] = {}
        self.helpers: dict[str, dict[str, str]] = {}
        self.test_names: dict[str, set[str]] = {}
        self.by_name: dict[str, list[str]] = collections.defaultdict(list)
        self.by_stem: dict[str, str] = {}
        for root, dirs, names in os.walk(CRATES):
            dirs[:] = [d for d in dirs if d != "target"]
            for name in names:
                if not name.endswith(".rs"):
                    continue
                path = os.path.relpath(os.path.join(root, name))
                in_tests = f"{os.sep}tests" in root or root.endswith(
                    os.sep + "tests"
                )
                # A src file is a witness host only when it carries an inline
                # `#[cfg(test)]` module; without one its fns are not tests.
                if not in_tests and "#[cfg(test)]" not in read(path):
                    continue
                found = fn_bodies(path)
                if not found:
                    continue
                self.tests[path] = found
                self.helpers[path] = found
                self.test_names[path] = test_fns(path)
                for fn_name in self.test_names[path]:
                    self.by_name[fn_name].append(path)
                # Only a tests/ file may answer a bare stem: two files named
                # `config.rs` must not shadow each other.
                if in_tests:
                    self.by_stem[name[:-3]] = path

    def resolve(self, cell: str) -> tuple[list[tuple[str, str]], list[str]]:
        """(path, fn) pairs the cell names, plus the tokens that resolve nothing."""
        cell = re.sub(r"\[[^\]]*\]", " ", cell).strip()
        resolved: list[tuple[str, str]] = []
        unresolved: list[str] = []
        for token in cell.split():
            token = token.strip("`'\"")
            if not token or token.lower().startswith("n/a"):
                continue
            if os.path.isfile(token):
                path = os.path.relpath(token)
                resolved.extend((path, fn) for fn in self.tests.get(path, {}))
                if path not in self.tests:
                    unresolved.append(token)
                continue
            if "::" in token:
                raw_path, name = split_witness(token)
                path = os.path.relpath(raw_path) if os.path.isfile(raw_path) else ""
                if path and name in self.tests.get(path, {}):
                    resolved.append((path, name))
                else:
                    unresolved.append(token)
                continue
            if token in self.by_name:
                resolved.extend((path, token) for path in self.by_name[token])
                continue
            if token in self.by_stem:
                path = self.by_stem[token]
                resolved.extend((path, fn) for fn in self.test_names[path])
                continue
            unresolved.append(token)
        return resolved, unresolved


def self_test() -> None:
    """The traps this audit fell into before it could be trusted.

    A witness that builds through a file-local helper is not a substrate test,
    and `Stream.ignore` is a claim about stream.rs, not about any `ignore` in
    the workspace. Both were false positives, so both stay pinned here.
    """
    types = {"Decimal"}
    fns = {"from_string"}
    helpers = {"dec": '{ Decimal::from_string("1") }'}
    assert references('assert!(dec("1").is_positive());', types, fns, helpers), (
        "a helper-wrapped port value must count as a port reference"
    )
    assert not references("assert_eq!(vec![1, 2], vec![1, 2]);", types, fns, {}), (
        "std-only assertions must not count"
    )
    assert references("let d: Decimal = todo!();", types, fns, {}), (
        "a port type in a signature must count"
    )
    assert split_witness("crates/x/tests/a.rs::row") == ("crates/x/tests/a.rs", "row"), (
        "a tests/ witness keeps its file and fn"
    )
    assert split_witness("crates/x/src/a.rs::tests::mod::row") == (
        "crates/x/src/a.rs",
        "row",
    ), "module segments between the file and the test are noise"


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--ledger", default=LEDGER)
    parser.add_argument("--artifact", default=ARTIFACT)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args(argv)
    if args.self_test:
        self_test()
        print("witness-audit: self-test passed")
        return 0

    types, fns = port_symbols()
    index = Witnesses()

    rows, errors = load_ledger(args.ledger)
    if errors:
        for error in errors:
            print(error)
        return 1

    ported = [r for r in rows if r[0] == "ported"]
    excused = [r for r in rows if r[0] == "not-applicable"]

    unresolved: list[tuple[list[str], str]] = []
    substrate: list[tuple[list[str], str]] = []
    clean = 0
    for row in ported:
        hits, missing = index.resolve(row[5])
        if not hits:
            unresolved.append((row, " ".join(missing) or row[5]))
            continue
        reached = set()
        for path, fn in hits:
            body = index.tests[path].get(fn, "")
            reached |= references(body, types, fns, index.helpers[path])
        if reached:
            clean += 1
        else:
            substrate.append((row, " ".join(missing) or row[5]))

    modules = module_files()
    all_src = [p for paths in modules.values() for p in paths]

    def defined(api: str, module: str, scoped: list[str]) -> bool:
        """Is `Module.api` defined where the reason says it looked?

        The scope matters more than the name: `Schema.Date does not exist` is
        not falsified by `Config::date`, and `Command.provide` is not
        falsified by an unrelated `provide` in another crate. So the lookup
        only reads files the module name points at, and a module with no file
        of its own is unproven rather than proven either way. Only functions
        count: a `RegExp` type in `data/regexp.rs` does not give `Schema` a
        `RegExp` constructor.
        """
        name = snake(api)
        paths = scoped or modules.get(snake(module), [])
        if not paths and scoped:
            paths = scoped
        elif not paths:
            needle = snake(module)
            # `Schema.Date` speaks for the adapter file of that name, and this
            # crate names adapters `schema_<thing>.rs`, so a stem lookup misses
            # every one of them: the blindness that let ten `Schema.Date does
            # not exist` reasons stand after the constructor landed.
            paths = [
                path
                for path in all_src
                if os.path.basename(path).startswith("schema")
            ] if needle == "schema" else [path for path in all_src if needle in path.lower()]
        for path in paths:
            if re.search(
                r"(?m)^[ \t]*(?:pub(?:\([^)]*\))?[ \t]+)?(?:async[ \t]+)?"
                r"fn[ \t]+" + re.escape(name) + r"\b",
                read(path),
            ):
                return True
        return False

    absent_resolved: list[tuple[list[str], str]] = []
    vague: list[tuple[list[str], str]] = []
    for row in excused:
        reason = row[5]
        # A reason that names the file it checked is read against that file, so
        # `Effect.forever is absent from effect.rs` is falsified by effect.rs
        # alone and not by a same-named helper in another crate.
        scoped = FILE_REF.findall(reason)
        for match in ABSENT.finditer(reason):
            if defined(match.group(2), match.group(1), scoped):
                absent_resolved.append((row, reason))
                break
        if not SPECIFIC.search(reason) and not ABSENT.search(reason):
            vague.append((row, reason))

    with open(args.artifact, "w", encoding="utf-8") as out:
        out.write("verdict\tpath\tline\twitness_or_reason\n")
        for row, why in unresolved:
            out.write(f"no-such-witness\t{row[1]}\t{row[2]}\t{why}\n")
        for row, why in substrate:
            out.write(f"substrate-only\t{row[1]}\t{row[2]}\t{why}\n")
        for row, why in absent_resolved:
            out.write(f"absent-but-defined\t{row[1]}\t{row[2]}\t{why}\n")
        for row, why in vague:
            out.write(f"vague-reason\t{row[1]}\t{row[2]}\t{why}\n")

    print(f"{args.ledger}: {len(rows)} rows, {len(ported)} ported, {len(excused)} excused")
    print(f"  ported, witness exercises port code: {clean}")
    print(f"  ported, witness not found:           {len(unresolved)}")
    print(f"  ported, witness never touches port:  {len(substrate)}")
    print(f"  excused, claimed API exists:         {len(absent_resolved)}")
    print(f"  excused, reason names nothing:       {len(vague)}")

    def show(label: str, items: list[tuple[list[str], str]], limit: int = 20) -> None:
        if not items:
            return
        print(f"\n{label} ({len(items)})")
        for path, count in collections.Counter(r[1] for r, _ in items).most_common(10):
            print(f"  {count:4d}  {path.replace('packages/effect/test/', '')}")
        print("  --")
        for row, why in items[:limit]:
            print(f"  {row[1]}:{row[2]}  {row[4][:56]}  -> {why[:60]}")

    show("PORTED WITHOUT AN EXISTING WITNESS", unresolved)
    show("PORTED ON A WITNESS THAT NEVER TOUCHES PORT CODE", substrate)
    show("EXCUSED, BUT THE CLAIMED-ABSENT API HAS A DEFINITION", absent_resolved)

    print(f"\nartifact: {args.artifact}")
    failed = len(unresolved) + len(substrate) + len(absent_resolved)
    if failed:
        print(f"FAIL: {failed} claims do not hold")
        return 1
    print(
        "OK: every ported witness exists and reaches port code; no excuse claims an "
        f"absent API that is defined ({len(vague)} reasons are narrative, reported above)"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
