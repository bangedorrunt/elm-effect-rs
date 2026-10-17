# 0004. Reconcile uncertain outcomes with the planner; back the loop log with a file

Status: proposed. `accepted` is the operator's sign-off (ADR-0001).

## Context

ADR-0002 L14 says an `Unknown` outcome has no default exit: only a reconcile pass may leave it, and a retry must reuse the key. The journal crate already carries the states (`Unknown`, `ReconcilePending`), the pure dispatch planner `plan_attempt`, and a generation-fenced claim. The elm loop had none of that. After a crash it reported in-flight commands and stopped there. Its only durable log was in memory, so nothing survived a restart.

## Decision

1. **An uncertain command is never re-run on its own.** Recovery marks a durable command whose intent exists but whose outcome is missing as in flight. It computes that command's `DispatchPlan` with the journal's `plan_attempt`, reading the command's `ProviderCapability`.
2. **The host decides when to reconcile.** `Loop::reconcile()` applies each plan. `ReconcileFirst` asks the provider through `Interpret::reconcile`, and `RetrySameKey` re-dispatches under the same key as the next attempt. Every other plan stays in flight. Reconcile is explicit because it can call a provider.
3. **Capability is declared by the command.** `Cmd::capability()` defaults to offering nothing, so an uncertain outcome is refused unless the command says otherwise. A provider with no contract is never retried blind.
4. **The loop's durable log is a file.** `FileLog` is a JSON-lines file with the same admission rules as `MemLog`. Each append is synced before it becomes visible. A torn final line, whose append never returned `Ok`, is cut off on open. A corrupt complete line fails the open.
5. **Compaction keeps the fencing state.** The rewritten file starts with a checkpoint line holding the sequence and epoch state. Kept records are replayed without the fence checks, because the checkpoint already covers them.

## Consequences

- A refused command stays in flight until a human or a reconcile query resolves it. `snapshot()` stays refused while anything is in flight.
- `FileLog` syncs each record. That is correct but slow. Batch the sync if throughput matters.
- The loop keeps its own records and does not use `JournalStore`. The row ledger stays the toron-facing store for later adoption. The two must not disagree silently, so adopting the row ledger is a separate decision.
- Ephemeral commands still leave no trace, by definition.

## Verification

- `crash_recovery.rs` covers the four plans: refuse, same-key retry, query-first, and a restart on a real file.
- `file_log.rs` covers reopen, fencing across restart, torn tail, corrupt line, and compaction with a checkpoint.
