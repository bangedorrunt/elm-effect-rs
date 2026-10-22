#!/usr/bin/env python3
"""Check the beads tracker's closes against git.

Why this exists: `br list` projects a thin record, so a close whose evidence
cannot be resolved is invisible there, and both failure directions have
already happened. Beads have sat open while their work landed under another
id (port-rcmap-3oph, carried by uw8p), and closes have pointed at commits
that name some other bead. Either way the board misleads: it reads as work
remaining, or as work finished, when the opposite is true.

One class is a hard defect: a close made AFTER the gate ceremony began with
no pass gate row. Everything else is a warning, because an abbreviated or
cross-repo sha can still be sound evidence, and the fence may be what
actually carries the close.

    python3 conformance/tracker-audit.py [--flywheel ~/workspace/flywheel]
"""

from __future__ import annotations

import argparse
import collections
import json
import os
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
BEADS = ROOT / ".beads"
HEX = set("0123456789abcdef")


def git(repo: Path, *args: str, stdin: str | None = None) -> str:
    return subprocess.run(
        ["git", "-C", str(repo), *args],
        input=stdin,
        capture_output=True,
        text=True,
        check=False,
    ).stdout


def load(name: str) -> list[dict]:
    path = BEADS / name
    if not path.exists():
        return []
    return [json.loads(line) for line in path.read_text().splitlines() if line.strip()]


def message(repo: Path, sha: str) -> str:
    """The commit message. Reads the object, not the refs: a close can name a
    commit that still exists but is no longer reachable from any branch."""
    return git(repo, "log", "-1", "--format=%B", sha)


def resolve(repo: Path, shas: list[str]) -> dict[str, str]:
    """Every abbreviated/full sha that names a commit here -> its full sha."""
    if not shas:
        return {}
    found = {}
    for sha in shas:
        full = git(repo, "rev-parse", "--verify", "--quiet", f"{sha}^{{commit}}").strip()
        if full:
            found[sha] = full
    return found


CLOSED_ROW = {"ported", "not-applicable", "skipped-upstream"}


def ledger() -> list[list[str]]:
    path = ROOT / "conformance/behavior-tests.tsv"
    if not path.exists():
        return []
    rows = [line.rstrip("\n").split("\t") for line in path.read_text().splitlines() if line.strip()]
    return [r for r in rows if len(r) > 3 and not r[0].startswith("#") and r[0] != "status"]


def check_ledger(rows: list[list[str]]) -> tuple[list[str], list[str]]:
    """The two claims a bead or module row can make that the ledger can falsify.

    A tombstone is a bead retired without ever being closed, so its title is
    the only statement of what it would have ported. If the test it names
    still has open rows, the retirement hid work rather than dropping it.

    A `matched` module with no ported row is parity theatre: every row was
    excused, so nothing was carried. An excused row cannot buy parity.
    """
    import collections

    open_rows: collections.Counter[str] = collections.Counter()
    ported: collections.Counter[str] = collections.Counter()
    for status, path, *_ in rows:
        if status not in CLOSED_ROW:
            open_rows[path] += 1
        elif status == "ported":
            ported[path] += 1

    hard, warn = [], []
    for issue in load("issues.jsonl"):
        if issue.get("status") != "tombstone":
            continue
        name = (issue.get("title") or "").removeprefix("Port ").split(" ")[0]
        hidden = [p for p in open_rows if p.rsplit("/", 1)[-1].startswith(name)]
        if hidden:
            hard.append(f"{issue['id']}: tombstoned but {hidden[0]} still has open rows")
        if not issue.get("close_reason"):
            warn.append(f"{issue['id']}: tombstoned with no reason recorded")

    modules_path = ROOT / "conformance/modules.toml"
    if modules_path.exists():
        import tomllib

        modules = tomllib.loads(modules_path.read_text())["module"]
        for module in modules:
            if module["status"] == "matched" and module.get("tests"):
                if sum(ported.get(t, 0) for t in module["tests"]) == 0:
                    hard.append(
                        f"{module['path']}: matched with no ported row (every row excused)"
                    )
        scope_hard = check_scope(modules, rows)
        hard += scope_hard
    return hard, warn


SCOPE = ROOT / "conformance/modules-without-rows.tsv"
SCOPE_REASONS = {"barrel", "no-test"}


def check_scope(
    modules: list[dict], rows: list[list[str]]
) -> list[str]:
    """Every module the ledger cannot speak for has to be named, and named once.

    `modules-without-rows.tsv` is a scope statement: a module with no behavior
    rows is either recorded there or invisible, and an invisible module is the
    one shape of remaining work this tracker can lose. The reverse direction
    matters as much, because a listed module that has since gained rows reads
    as a hole that is already filled.
    """
    row_paths = {r[1] for r in rows if len(r) > 1}
    untracked = {
        m["path"] for m in modules if not any(t in row_paths for t in (m.get("tests") or []))
    }
    recorded: dict[str, str] = {}
    duplicates: list[str] = []
    if SCOPE.exists():
        for line in SCOPE.read_text().splitlines():
            if line.startswith("#") or not line.strip():
                continue
            fields = line.split("\t")
            path = fields[0]
            if path in recorded:
                duplicates.append(path)
            recorded[path] = fields[1] if len(fields) > 1 else ""

    hard: list[str] = []
    for path in sorted(untracked - set(recorded)):
        hard.append(f"{path}: no behavior row and not recorded in modules-without-rows.tsv")
    for path in sorted(set(recorded) - untracked):
        hard.append(f"{path}: recorded with no rows, but it now carries rows (stale entry)")
    for path in sorted(set(duplicates)):
        hard.append(f"{path}: recorded twice in modules-without-rows.tsv")
    for path, reason in sorted(recorded.items()):
        if not reason.strip():
            hard.append(f"{path}: recorded with no reason")
        elif reason not in SCOPE_REASONS:
            hard.append(f"{path}: reason {reason!r} is not one of {sorted(SCOPE_REASONS)}")
        elif reason == "barrel" and not path.endswith("/index.ts"):
            hard.append(f"{path}: recorded as a barrel, but its path is not an index.ts")
    return hard


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--flywheel",
        default=os.environ.get("FLYWHEEL_REPO", str(Path.home() / "workspace/flywheel")),
        help="sibling repo whose commits a bead here may name",
    )
    args = parser.parse_args()
    flywheel = Path(args.flywheel)

    issues = load("issues.jsonl")
    gates = load("gates.jsonl")
    closed = [i for i in issues if i.get("status") == "closed"]
    if not closed:
        print("tracker-audit: no closed beads")
        return 0

    gated = {g["issue_id"] for g in gates if g.get("passed") and g.get("to_status") == "closed"}
    first_gate = min((g["recorded_at"] for g in gates if g.get("recorded_at")), default="")

    shas = list({i["commit_sha"] for i in closed if i.get("commit_sha") and set(i["commit_sha"]) <= HEX})
    local = resolve(ROOT, shas)
    elsewhere = {} if not flywheel.exists() else resolve(flywheel, [s for s in shas if s not in local])

    hard: list[str] = []
    warn: list[str] = []
    for issue in closed:
        iid, sha = issue["id"], issue.get("commit_sha") or ""
        closed_at = (issue.get("closed_at") or "")[:19].replace("T", " ")
        late = bool(first_gate) and closed_at > first_gate

        if not sha:
            hard.append(f"{iid}: closed with no commit_sha")
        elif set(sha) - HEX:
            hard.append(f"{iid}: commit_sha {sha!r} is not a sha")
        elif full := local.get(sha):
            if iid not in message(ROOT, full):
                warn.append(f"{iid}: {sha[:8]} exists but its message does not cite this bead")
            elif len(sha) < 40:
                warn.append(f"{iid}: abbreviated sha {sha} (resolves to {full[:8]})")
        elif full := elsewhere.get(sha):
            # Cross-repo evidence is sound when the message cites the bead; the
            # citation has to be read from the repo that owns the commit.
            if iid not in message(flywheel, full):
                warn.append(f"{iid}: {sha[:8]} is a {flywheel.name} commit that does not cite this bead")
        else:
            warn.append(f"{iid}: {sha[:8]} resolves in neither repo; its fence must carry the close")

        if iid not in gated:
            if late:
                hard.append(f"{iid}: closed after the gate ceremony began with no pass gate row")
            else:
                warn.append(f"{iid}: pre-ceremony close, no gate row possible")
        if not issue.get("close_verdict"):
            warn.append(f"{iid}: no close_verdict recorded")

    ledger_hard, ledger_warn = check_ledger(ledger())
    hard += ledger_hard
    warn += ledger_warn

    statuses = collections.Counter(i.get("status") for i in issues)
    verdicts = collections.Counter(i.get("close_verdict") for i in closed)
    print(f"closed: {len(closed)}   other statuses: {dict(statuses)}")
    print(f"gate rows: {len(gates)}   first: {first_gate or '(none)'}")
    print("verdicts:", dict(verdicts.most_common()))
    print(f"sha resolves: {len(local)} here, {len(elsewhere)} in {flywheel.name}")
    print(f"hard defects: {len(hard)}   warnings: {len(warn)}")
    for line in hard:
        print("  FAIL", line)
    for line in warn:
        print("  warn", line)
    return 1 if hard else 0


if __name__ == "__main__":
    sys.exit(main())
