---
tier: perishable
tags: [conformance, ledger, reservations, toron, lease, multi-lane]
bead: elm-effect-rs-port-jsonschema-dialects-8wm0
created: 2026-10-14
verify: sh -c "grep -c '^packages/effect/test/JsonSchema.test.ts' conformance/queued-ledger-edits.tsv | grep -qx 14"
verify_result: ok (14 queued rows; the ledger file itself carries none of them)
---

# A refused reservation still exits 0

## The failure

I acquired `conformance/behavior-tests.tsv`, flipped 22 rows, committed, and
released the lease. Then I re-acquired for the next batch like this:

```sh
toron reserve acquire --project elm-effect-rs --as CalmHarbor --paths "$PATHS" --json | grep granted
```

The output was `"conflicts": [BoldKite], "granted": []`. **The command still
exited 0**, so the `&&` before `flip-row.py` ran it anyway and wrote 14 of my
rows into a file another agent held exclusively. He was, at that moment,
sitting on 30 uncommitted TxHashMap flips.

The write happened to be harmless: `flip-row.py` reads the file, edits the
rows it was given, and rewrites everything else untouched, so his 30 rows came
back byte-identical. It was harmless **by luck, not by construction**. If he
had written the file from a snapshot read before my flip, my rows would have
vanished silently, and if he had staged the whole file, my rows would have
ridden into his commit.

## The repair

Reverted only my 14 lines in place (status back to `unported`, rust cell
cleared), left his 30 exactly as they were, and verified with the added-line
paths: the diff carries `TxHashMap.test.ts` and nothing else. Then staged the
14 rows in `conformance/queued-ledger-edits.tsv`, which is the mechanism for
exactly this, and confirmed with `flip-row.py --from-queue <file> --check`.

## What to do instead

1. Read the acquire result, never the exit status:
   `toron reserve acquire ... --json | python3 -c "import json,sys; d=json.load(sys.stdin); sys.exit(0 if d['granted'] else 1)"`
2. Or ask the roster directly before the write:
   `toron reserve conflicts --project <p> --as <pin> --paths <glob>`.
3. Keep the acquire and the write in separate commands. A refused lease has to
   stop you; an `&&` chain after a command that cannot fail does not.
4. Do not "clean up" a shared file you wrote without a lease by reverting the
   whole file. Revert your own lines surgically and check the paths of the
   remaining diff, or you delete a sibling lane's uncommitted work.

## Principles

- **boundary-discipline** — a shared file under someone else's lease is their
  boundary; a lease request that was *denied* is not permission.
- **prove-it-works** — the acquire's own JSON, read and asserted, is the proof;
  exit 0 only proves the CLI ran.
- **never-silently** — the write, the revert, and the queue entry are all
  recorded in the bead, because a near-miss that stays quiet is the one that
  becomes a loss next time.
