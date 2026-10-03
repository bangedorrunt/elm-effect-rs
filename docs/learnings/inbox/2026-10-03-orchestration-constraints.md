---
tier: perishable
tags: [bd-iqte, orchestration, flywheel, loop-mode, dispatch-cap]
bead: elm-effect-rs-epic-elm-effect-port-0c6.7
wave: 2
created: 2026-10-03
verify: flywheel loop-mode run elm-effect-rs --work --force --once
verify_result: ok (single pass dispatches; durable mode panics)
---
# Orchestration constraints observed on elm-effect-rs (2026-10-03)

## Durable loop-mode is unusable on this build
`flywheel loop-mode run <p> --work` (durable) panics per iteration
(`Cannot start a runtime from within a runtime`) and, on a resumed invocation,
storms serialization errors from stale checkpoints. `--once --force` passes are
clean and are the working door. Do not resume a killed durable loop without
`--force`.

## The 4096-byte dispatch cap kills profiled panes
The loop brief template is ~3.2KB with long bead ids; a role behavior preamble
adds ~1.4-1.5KB, so every profiled pane fails `over the 4096 byte cap (NOT
sent)`. Fix used here: strip `profile` from the spawn-registry entry
(`~/.local/state/flywheel/agents/<project>-<hash>.json`) for loop-managed panes,
or spawn bare (`--pi N --model <pin>`), so sends are raw and fit.

## Claims must use the PIN, not the herdr name
`reclaim_stranded` recognizes a claim as live only when the assignee matches a
herdr name in the live set OR a registry PIN whose pane is live. Claims written
with the herdr name (`crew-...-coder`) read as stranded and get yanked and
re-dispatched every pass. Claim with `--assignee <pin>` (SlateHerder).

## Spawn joins, it does not add
`flywheel spawn --join` only attaches and arrives; it does not create panes.
`flywheel add <project> --pi N` extends a live swarm (new panes land in the
existing workspace).

## Pane identity can drift from the spawn registry
Compose-spawned panes self-arrived with pins the registry did not name
(coder acts as SlateHerder; registry said SilverPond; planner arrived as the
captain pin until corrected). Trust the pane's own arrive + commits; fix
impersonation loud, leave self-consistent drift alone.

## Collisions self-heal through leases
When two panes target one bead, the lease holder wins; the other stands down
after a lease conflict. Keep the room informed and check `br show <id>`
assignee plus `toron reserve list` before starting.
