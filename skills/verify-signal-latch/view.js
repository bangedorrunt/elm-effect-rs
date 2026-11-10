export const title = "Signal Latch, Park Commit Gap, and Liveness Invariant";

export const state = (v) => ({
  Runner: v.runner_pc === "Done" ? `Done(${v.outcome})` : v.runner_pc,
  EnginePhase: v.phase,
  PendingSignals: v.signals > 0 ? "HasSignals" : "NoSignals",
});

export const step = (action, from, to) => {
  switch (action) {
    case "RunnerCheckPre":
      return from.phase === "Cancelled"
        ? "runner rejects cancelled run at precheck"
        : (from.signals > 0 ? "runner consumes latched signal early" : "runner sees no signals");
    case "RunnerCommitPark":
      return from.phase === "Cancelled"
        ? "runner detects cancel during park"
        : "runner commits park in storage";
    case "RunnerGapCheck":
      return from.signals > 0
        ? (from.phase === "Cancelled" ? "runner detects cancel in gap & fails terminal" : "runner consumes gap signal & wakes")
        : "runner confirms no gap signals & parks";
    case "SignallerSignal":
      return "external signal arrives";
    case "CancellerCancel":
      return "external cancel arrives";
    case "TerminalDone":
      return null;
    default:
      return action;
  }
};
