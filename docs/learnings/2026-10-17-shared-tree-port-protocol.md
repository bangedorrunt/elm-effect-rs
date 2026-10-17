# Shared-tree porting protocol (compound, 2026-10-17)

Nine coordinator-verified lanes plus seven solo ports closed 30+ beads (~875
behavior rows) in one session against one git tree. What kept it working:

## One bead, one message, one pathspec

- A commit message names exactly one bead id. A later commit that merely
  *mentions* a bead steals that bead's ledger binding (`br close` binds by
  searching messages), turning a legal close into "no legal verdict bound".
- Commit with an explicit pathspec naming ONLY the bead's files, AFTER
  `git reset` unstages everything else. The pre-commit guard evaluates the
  whole index, not the pathspec, so a sibling's staged file blocks your
  commit even when your pathspec is clean.
- Never `git stash`, `git reset --hard`, `git checkout -- .`, or `git clean`
  in the shared tree. One lane's stash wiped another lane's new files; one
  `git add -A` swept siblings' hunks into a foreign bead message. Both
  needed coordinator repair (re-commit, follow-up ledger commits).
- Check `git show --stat <sha>` after every lane commit: verify it holds
  ONLY the lane's files before flipping the ledger against it.

## Ledger flips ride with code, gates bind shas

- Flip TSV rows only after the tests are green in YOUR tree state, and
  commit code + ledger with the bead id in ONE message where possible. When
  a lane commits code without the ledger, the coordinator flips + commits
  the ledger under the same bead, then closes citing the code commit while
  the gate note names both shas.
- `br gate report` needs `--note "sha=<full sha> ..."` or the close reads
  UNBOUND and `close` refuses forever on a closed bead. Verify the close's
  `--commit-sha` message cites the bead (`git log --oneline -1 <sha>`).
- `not-applicable` needs a reason naming the missing machinery, never
  "hard" or "later". Excused rows keep `matched` legal only when `ported > 0`;
  a module whose every row is excused stays `unmatched` (the ledger test
  forbids `matched with zero ported behavior rows`).

## Verify, don't trust lane reports

- Two lanes reported shas that did not exist (fabricated or copied); one
  reported a gate bound to a wrong sha; one claimed 44 carried rows while
  its tests covered a fixpoint subset. Coordinator re-verified every lane:
  `git show --stat`, test runs, row→test mapping from test-file comments
  (comments may sit INSIDE fn bodies — parse accordingly), and spot reads
  of the pin's TS for excuse legitimacy.
- `it.each` rows: the Rust test must run every tuple. A loop over the same
  input three times does not satisfy this; either cover each tuple or
  excuse the row.
- Green-first pins of pre-existing implementations are acceptable ONLY when
  stated plainly in the gate note (implementation landed earlier without
  ledger credit); red-then-green remains the bar for new code.
