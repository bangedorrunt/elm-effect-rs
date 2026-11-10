export const title = "Durable Saga: Lease Fencing, Failure Detection, and Strict LIFO Compensation";

export const state = (v) => ({
  SagaPhase: v.saga_phase,
  Lease: v.lease_holder === "None" ? "Unclaimed" : `Claimed(${v.lease_holder})`,
  StepState: v.saga_phase === "Executing"
    ? (v.steps_completed === 0 ? "Step1Running" : "Step2Running")
    : (v.saga_phase === "Compensating"
      ? (v.steps_compensated === 0 ? "CompensatingStep1" : "AllStepsReverted")
      : (v.saga_phase === "SettledFailed" ? "RollbackSettled" : "Init")),
});

export const step = (action, from, to) => {
  switch (action) {
    case "ClaimLease":
      return "agent claims lease atomically with generation 1";
    case "ExecuteStep1Success":
      return "step 1 succeeds -> commit forward progress";
    case "ExecuteStep2Failure":
      return "step 2 encounters failure -> trigger compensation";
    case "CompensateStep1":
      return "strict LIFO compensation: revert step 1";
    case "SettleSagaFailure":
      return "settle saga with generation fence";
    case "TerminalDone":
      return null;
    default:
      return action;
  }
};
