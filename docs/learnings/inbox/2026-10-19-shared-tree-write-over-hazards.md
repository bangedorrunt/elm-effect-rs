---
tier: perishable
tags: [elm-effect-rs, port-protocol, ledger, incidents]
bead: elm-effect-rs-v3bh
created: 2026-10-19
verify: sh -c "cargo test -p elm-effect-schema --test schema_named_checks && cargo test -p elm-effect-schema"
verify_result: unit-test-verified (sha=f270151; fence green)
---

# Two ways this lane wrote over another lane's work, and the checks that catch them

Both landed on 2026-10-19 in the shared tree, both repaired in the same session,
and both are cheaper to prevent than to repair.

**A new test file's name is not free: look for the file first.** A witness file
was named `tests/schema_checks.rs` after the ledger subject it carries
("Checks"), and `write_file` replaced an existing 331-line `schema_checks.rs`
owned by two other beads. The commit (`dd590d5`) deleted 302 lines of another
lane's witnesses, and the ledger's `rust` column kept pointing at functions that
no longer existed. Repair: restore from `cli^`, move the new witnesses to a name
nobody owns (`schema_named_checks.rs`), verify the restore with
`git show <parent>:<path> | diff -q - <path>` before believing it. `write_file`
replaces the whole file; `str_replace` or an `ls`/`glob` first would have cost
seconds.

**A ledger line number is the pin's `it(...)` line, not the title above it.** The
`UnknownFromJsonString` block in `Schema.test.ts` starts one row earlier than a
reader expects, so three rows were flipped with the witness of the row next door
(`:5991` is `URLFromString`, `:6011` is the Unknown round-trip, `:6030` is the
from-json-string serializer, `:6071` is the nested one). Before flipping a batch,
print the row's own title beside the line number you are about to cite, and read
the title — not the line's neighbourhood.
