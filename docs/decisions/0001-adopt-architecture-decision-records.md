---
status: proposed
date: 2026-10-02
decision-makers: operator of the flywheel, toron, beads, and chiebukuro stack
consulted: none
informed: agents implementing elm-effect-rs
---

# Adopt architecture decision records

## Context and Problem Statement

`elm-effect-rs` is a new repository. The work it exists to hold, a Rust port of Effect 4.0.0 plus an Elm loop for four existing codebases, will be implemented by agents across many sessions. A decision that lives only in a chat transcript is invisible to the next agent. How do we record decisions so a later agent can implement them without re-deriving them?

## Decision Drivers

* Agents implement from documents, not from memory of a session.
* The port is hard to reverse once sibling crates depend on it.
* Several real alternatives exist (see ADR-0002) and the rejection reasons have to survive.

## Considered Options

* Architecture decision records in `docs/decisions/`, MADR-shaped, numeric filenames.
* Decisions only in crate docs and code comments.
* Decisions only in the sibling repos' existing ADR sets.

## Decision Outcome

Chosen option: "Architecture decision records in `docs/decisions/`", because the port is a new codebase with no crate yet, and the siblings must not become the source of truth for a framework they will later depend on.

Records are Markdown with YAML front matter (`status`, `date`, `decision-makers`, `consulted`, `informed`). Filenames are `NNNN-verb-phrase.md`. Status values are `proposed`, `accepted`, `rejected`, `deprecated`, and `superseded by ADR-NNNN`. `accepted` is the operator's sign-off. `proposed` means that word has not been flipped. It does not mean stop. When a record contains an agent-authority section, an agent implements what that section authorizes, including while the status is `proposed`. A record with no such section is a spec to review, and an agent does not write the code it describes until the status is `accepted`. Superseding creates a new record and links both ways. History is not rewritten. Learnings are appended under `## More Information` with a date.

Code that implements an accepted record names it in a comment at the entry point: `// ADR-0002: ...` and the path. One comment per entry point.

### Consequences

* Good, because the next agent can read one directory and know what is decided.
* Bad, because every architectural change now costs a document before code.
* Neutral, because sibling repos keep their own ADR directories. This repo does not absorb them.

## Implementation Plan

* **Affected paths**: `docs/decisions/`, `docs/decisions/README.md`, this file, `docs/decisions/0002-port-effect-4-into-rust-with-an-elm-loop.md`.
* **Dependencies**: none.
* **Patterns to follow**: MADR sections (context, drivers, options, outcome, consequences) plus an Implementation Plan and checkbox Verification, as in ADR-0002.
* **Patterns to avoid**: a decision that exists only as a crate-level comment. A status of `accepted` set by an agent without the operator.
* **Configuration**: none.
* **Migration steps**: none. The repository had no records before this file.

### Verification

- [ ] `docs/decisions/README.md` lists every record in this directory with its status.
- [ ] No record in this directory has an empty section that the template marks as required.
- [ ] An agent that opens the repository can find ADR-0002 from the root `README.md`.

## Pros and Cons of the Options

### Architecture decision records in `docs/decisions/`

* Good, because the format is already used by chiebukuro (`docs/decisions/0011-chiebukuro-daemon-resonate.md`) and by flywheel's governed-by comments.
* Good, because a proposed record can sit unaccepted without becoming code.
* Bad, because the set will grow and the index has to be maintained.

### Decisions only in crate docs and code comments

* Good, because the decision sits next to the code.
* Bad, because there is no code yet, and comments do not record rejected alternatives.

### Decisions only in the sibling repos

* Good, because those repos already have ADR habits.
* Bad, because the framework would be specified in four places, and a dependency from those repos back to a spec they host is how cycles start.

## More Information

The first architectural record is [ADR-0002](0002-port-effect-4-into-rust-with-an-elm-loop.md). Neither record is `accepted` until the operator says so.
