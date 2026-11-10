export const title = "Statechart Microstep: LCA Resolution, Law L1 Execution Order, and Eventless Always Quiescence";

export const state = (v) => ({
  MicrostepPhase: v.phase,
  ActiveLeaf: v.active_state,
  MicrostepCount: "Count_" + v.microstep_count,
});

export const step = (action, from, to) => {
  switch (action) {
    case "ReceiveTurnOn":
      return "external TurnOn event matched";
    case "ExitSourceState":
      return "LCA resolution: exit source state deepest-first";
    case "ExecuteTransitionActions":
      return "execute transition actions & context assigns";
    case "EnterTargetState":
      return "enter target state and resolve initial child";
    case "CascadeAlwaysMicrostep":
      return "eval always transition: cascade microstep";
    case "SettleQuiescence":
      return "all transient conditions false: settle at quiescence";
    case "TerminalStep":
      return null;
    default:
      return action;
  }
};
