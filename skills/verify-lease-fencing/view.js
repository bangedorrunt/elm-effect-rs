export const title = "Lease Reclaim, Generation Fencing, and Split-Brain Elimination";

export const state = (v) => ({
  Journal: `${v.journal_owner}[G${v.journal_gen}:${v.journal_state}]`,
  WorkerA: `${v.workerA_state}[G${v.workerA_gen}]`,
  WorkerB: `${v.workerB_state}[G${v.workerB_gen}]`,
  Time: `t=${v.clock}/DL=${v.journal_deadline}`,
});

export const step = (action, from, to) => {
  switch (action) {
    case "WorkerAClaim":
      return `WorkerA claims lease at gen ${to.journal_gen}`;
    case "WorkerAHeartbeat":
      return `WorkerA renews lease deadline to ${to.journal_deadline}`;
    case "WorkerAStall":
      return "WorkerA stalls (thread freeze/network lag)";
    case "AdvanceClock":
      return `clock advances to t=${to.clock}`;
    case "WorkerBReclaimAfterExpiry":
      return `WorkerB reclaims expired lease at gen ${to.journal_gen}`;
    case "WorkerAAttemptSettle":
      return to.workerA_state === "Committed"
        ? "WorkerA commits settlement"
        : "WorkerA settlement rejected with StaleGeneration (fenced)";
    case "WorkerBAttemptSettle":
      return to.workerB_state === "Committed"
        ? "WorkerB commits settlement"
        : "WorkerB settlement rejected with StaleGeneration (fenced)";
    case "TerminalDone":
      return null;
    default:
      return action;
  }
};
