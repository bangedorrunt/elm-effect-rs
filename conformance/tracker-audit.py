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
