export const title = "Structured Concurrency: Scope Finalization and Bounded Straggler Quarantine";

export const state = (v) => ({
  Scope: v.scope_state,
  Fiber: v.fiber_state,
  Deadline: v.deadline_hit ? "Expired" : "Pending",
});

export const step = (action, from, to) => {
  switch (action) {
    case "ForkChildFiber":
      return "fork child fiber attached to parent scope";
    case "FiberNormalComplete":
      return "child fiber runs to clean exit";
    case "ScopeCloseNormal":
      return "scope initiates normal close";
    case "DeadlineTrip":
      return "bounded deadline fires";
    case "InterruptOnDeadline":
      return "scope initiates close and signals interrupt";
    case "FiberResponsiveInterruptYield":
      return "responsive fiber yields at interrupt boundary";
    case "FiberUnresponsiveStraggler":
      return "unresponsive fiber quarantined & named by ID";
    case "RunLIFOFinalizers":
      return "execute finalizer in strict LIFO order";
    case "FinishScopeClose":
      return "all finalizers settled; scope closed cleanly";
    case "TerminalDone":
      return null;
    default:
      return action;
  }
};
