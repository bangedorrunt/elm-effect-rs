#!/usr/bin/env python3
"""Seed the ADR-0002 wave beads from docs/plan/bead-briefs.md.

Run ONCE on a fresh tracker (br init already done). Idempotence is not the
goal; re-running creates duplicates. Provenance, not a migration tool.

Parses: TITLE, PRIORITY, TYPE, LABELS, VERIFY, PRINCIPLES ('; '-'separated'),
optional DEPENDS handled below, and everything after 'DESCRIPTION:' as the body.
Creates each bead with --description-file and links children to the epic.
"""
import re
import subprocess
import sys
from pathlib import Path

SLUG = re.compile(r"^## ([a-z0-9][a-z0-9-]*)$")

ROOT = Path(__file__).resolve().parents[2]
BRIEFS = ROOT / "docs" / "plan" / "bead-briefs.md"
BR = "br"

# Intra-wave dependency edges (child <- parent). Cross-wave order is enforced
# by the wave gates at dispatch time.
DEPS = {
    "w1-effect-core": ["w1-kernel-skeleton"],
    "w1-fiber": ["w1-effect-core"],
    "w1-scope": ["w1-fiber"],
    "w1-runtime": ["w1-scope"],
    "w2-layer": ["w2-requirements"],
    "w2-wiring": ["w2-layer"],
    "w3-schedule": ["w3-clock"],
    "w3-policy": ["w3-schedule"],
    "w3-hedge": ["w3-clock"],
    "w3-datetime-cron": ["w3-clock"],
    "w6-partition": ["w6-all"],
    "w6-fiber-sets": ["w6-all"],
    "w7-channel-sink": ["w7-stream-core"],
    "w7-stream-concurrency": ["w7-stream-core"],
    "w8-schema-transform": ["w8-schema-core"],
    "w8-schema-tagged": ["w8-schema-transform"],
    "w8-schema-corpus": ["w8-schema-tagged"],
    "w10-elm-replay": ["w10-elm-program"],
    "w10-elm-lints": ["w10-elm-program"],
    "w11-journal-plan": ["w11-journal-golden"],
    "w11-journal-park": ["w11-journal-golden"],
    "w11-journal-store": ["w11-journal-golden"],
    "w12-log-epoch": ["w12-log-append"],
    "w12-log-gone": ["w12-log-epoch"],
    "w13-otel": ["w13-observability"],
    "w17-cluster-rest": ["w17-cluster-core"],
}

META = ("TITLE", "PRIORITY", "TYPE", "LABELS", "VERIFY", "PRINCIPLES")


def parse(path: Path):
    text = path.read_text()
    sections = []
    current = None
    in_desc = False
    for line in text.splitlines():
        match = SLUG.match(line)
        if match:
            slug = match.group(1)
            if slug.startswith("conventions"):
                current = None
                continue
            current = {"slug": slug, "desc": []}
            sections.append(current)
            in_desc = False
            continue
        if current is None:
            continue
        if not in_desc and line.startswith("DESCRIPTION:"):
            in_desc = True
            rest = line[len("DESCRIPTION:"):].strip()
            if rest:
                current["desc"].append(rest)
            continue
        if not in_desc and ":" in line:
            key, _, value = line.partition(":")
            key = key.strip()
            if key in META:
                current[key] = value.strip()
                continue
        if in_desc:
            current["desc"].append(line)
    return sections


def run(args, **kw):
    result = subprocess.run(args, check=True, capture_output=True, text=True, **kw)
    return result.stdout.strip()


def main():
    sections = parse(BRIEFS)
    epic = next((s for s in sections if s["slug"] == "epic-elm-effect-port"), None)
    if epic is None:
        sys.exit("epic section missing")

    ids = {}

    def create(section, parent=None):
        desc = "\n".join(section["desc"]).strip() + "\n"
        with open("/tmp/bead-desc.md", "w") as handle:
            handle.write(desc)
        args = [
            BR, "create", section["TITLE"],
            "--slug", section["slug"],
            "--description-file", "/tmp/bead-desc.md",
            "--verify", section["VERIFY"],
            "-p", section["PRIORITY"],
            "-t", section["TYPE"],
            "-l", section["LABELS"],
            "--silent",
        ]
        principles = re.split(
            r"; (?=[a-z][a-z0-9-]* — )", section["PRINCIPLES"]
        )
        for principle in principles:
            principle = principle.strip()
            if principle:
                args += ["--principle", principle]
        wave = None
        for label in section["LABELS"].split(","):
            if label.startswith("wave-") and label[5:].isdigit():
                wave = label[5:]
        if wave is not None:
            args += ["--wave", wave]
        if parent:
            args += ["--parent", parent]
        out = run(args, cwd=ROOT)
        return out.splitlines()[-1].strip()

    ids[epic["slug"]] = create(epic)
    print(f"epic: {ids[epic['slug']]}")

    for section in sections:
        if section["slug"] == epic["slug"]:
            continue
        ids[section["slug"]] = create(section, parent=ids[epic["slug"]])
        print(f"{section['slug']}: {ids[section['slug']]}")

    for child, parents in DEPS.items():
        for parent in parents:
            run([BR, "dep", "add", ids[child], ids[parent]], cwd=ROOT)
            print(f"dep: {ids[child]} <- {ids[parent]}")


if __name__ == "__main__":
    main()
