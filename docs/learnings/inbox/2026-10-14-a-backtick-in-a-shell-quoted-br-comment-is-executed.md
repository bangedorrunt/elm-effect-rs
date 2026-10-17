---
tier: perishable
tags: [beads, br, shell, quoting, record-keeping, workflow]
bead: elm-effect-rs-port-schema-a2c
created: 2026-10-14
verify: sh -c "cd /Users/bangedorrunt/workspace/elm-effect-rs && br comments elm-effect-rs-port-schema-a2c --json | grep -q 'pub fn flip'"
verify_result: ok (exit 0; the corrected comment stores the code fragment that the first attempt lost)
---

# A backtick in a shell-quoted `br comments add` is executed, and the record is silently wrong

## The failure

I recorded a queue comment on a bead with `br comments add <id> -m "...text..."`.
Inside the double-quoted argument I used backticks to quote code, as in
``Each body is `const schema = Schema.X; if (verifyGeneration) { ... }` ``.

Bash ran that as a command substitution. The comment was accepted, printed
"Draft saved", and stored **with the code fragments silently deleted**:
"Each body is  and nothing else", and "Schema." where `Schema.flip` was. One
fragment errored loudly on stderr (`syntax error near unexpected token '{'`,
`fn: command not found`) and the other two vanished without a murmur.

Nothing rejected it. A mangled handoff record reads exactly like a correct one.

## The rule

Write long bead comments and mail bodies to a file and pass `--file`:

    br comments add <bead> --actor <pin> --author <pin> --file /tmp/comment.md

Verified safe for `br comments add --file`. The same hazard is present anywhere
the text travels through a shell-quoted `-m` or `--body` argument, because the
question is never "did the command succeed" but "did the bytes arrive".

## How to catch it (do this, not just avoid it)

Read the record back and grep for the fragments that carry the meaning:

    br comments <bead> --json | python3 -c "import json,sys; ..."

Check the specific things that would be gone if the shell ate them: a function
name, a schema name, a number. Length alone is not enough, and neither is the
"Comment added" line.

The first comment was already mangled before I noticed, so the fix was a second
comment that states it replaces the broken one and keeps both facts (that the
shell eats backticks, and the queue itself).
